//! Owner diagnostic files. The lifecycle installs this sink only after election.

use crate::daemon::diagnostics::{DiagnosticSink, DiagnosticSource};
use crate::daemon::paths::effective_uid;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub const MAX_LOG_BYTES: u64 = 1024 * 1024;

pub struct RotatingLogSink {
    directory: PathBuf,
    active: Option<File>,
    active_bytes: u64,
}

impl RotatingLogSink {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            active: None,
            active_bytes: 0,
        }
    }

    fn active_path(&self) -> PathBuf {
        self.directory.join("daemon.log")
    }
    fn previous_path(&self) -> PathBuf {
        self.directory.join("daemon.log.1")
    }

    fn check_private_file(path: &Path) -> io::Result<Option<fs::Metadata>> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if !metadata.file_type().is_file()
            || metadata.uid() != effective_uid()
            || metadata.permissions().mode() & 0o777 != 0o600
            || metadata.len() > MAX_LOG_BYTES
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "unsafe diagnostic log",
            ));
        }
        Ok(Some(metadata))
    }

    fn open_active(&mut self) -> io::Result<()> {
        let path = self.active_path();
        let existing = Self::check_private_file(&path)?;
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .open(&path)?;
        let metadata = file.metadata()?;
        if !metadata.file_type().is_file()
            || metadata.uid() != effective_uid()
            || metadata.permissions().mode() & 0o777 != 0o600
            || metadata.len() > MAX_LOG_BYTES
            || existing
                .as_ref()
                .is_some_and(|prior| prior.dev() != metadata.dev() || prior.ino() != metadata.ino())
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "diagnostic log changed",
            ));
        }
        self.active_bytes = metadata.len();
        self.active = Some(file);
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        if let Some(file) = self.active.take() {
            file.sync_data()?;
        }
        let previous = self.previous_path();
        Self::check_private_file(&previous)?;
        match fs::remove_file(&previous) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs::rename(self.active_path(), previous)?;
        self.open_active()
    }
}

impl DiagnosticSink for RotatingLogSink {
    fn install(&mut self) -> io::Result<()> {
        let metadata = fs::symlink_metadata(&self.directory)?;
        if !metadata.file_type().is_dir()
            || metadata.uid() != effective_uid()
            || metadata.permissions().mode() & 0o777 != 0o700
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "unsafe diagnostic directory",
            ));
        }
        Self::check_private_file(&self.previous_path())?;
        self.open_active()
    }

    fn write(&mut self, _source: DiagnosticSource, fragment: &[u8]) -> io::Result<()> {
        if self.active.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "diagnostic log is not installed",
            ));
        }
        let mut remaining = fragment;
        while !remaining.is_empty() {
            if self.active_bytes == MAX_LOG_BYTES {
                self.rotate()?;
            }
            let count = remaining
                .len()
                .min((MAX_LOG_BYTES - self.active_bytes) as usize);
            self.active
                .as_mut()
                .unwrap()
                .write_all(&remaining[..count])?;
            self.active_bytes += count as u64;
            remaining = &remaining[count..];
        }
        Ok(())
    }

    fn drain(&mut self) -> io::Result<()> {
        self.active
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "diagnostic log is closed"))?
            .sync_data()
    }

    fn close(&mut self) -> io::Result<()> {
        if let Some(file) = self.active.take() {
            file.sync_data()?;
        }
        Ok(())
    }
}
