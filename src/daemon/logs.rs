//! Owner diagnostic files. The lifecycle installs this sink only after election.

use crate::daemon::diagnostics::{DiagnosticSink, DiagnosticSource};
use crate::daemon::paths::{InstancePaths, effective_uid};
use crate::protocol::time::Clock;
use std::collections::{HashMap, hash_map::Entry};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

/// Sink for lane failures (`WorkerStatus::record_failure`).
/// `RateLimitedLaneLog` is the daemon's implementation.
pub trait LaneErrorLog: Send + Sync {
    fn record(&self, lane: crate::service::kicks::Lane, error: &crate::protocol::results::ApiError);

    /// Like [`record`](Self::record), with the lane Pacer's consecutive-failure
    /// count (0 when the lane has no Pacer or has not backed off).
    fn record_with_attempt(
        &self,
        lane: crate::service::kicks::Lane,
        error: &crate::protocol::results::ApiError,
        _attempt: u32,
    ) {
        self.record(lane, error);
    }

    /// A sent wake prompt whose submission could not be checked or stayed
    /// unsubmitted after one submit-key retry. Other verification results
    /// are not reported. The default records nothing.
    fn record_wake_verification(
        &self,
        _seat: &crate::protocol::ids::SeatId,
        _verification: crate::scheduler::SubmissionVerification,
    ) {
    }
}

/// The default lane error log: records nothing.
pub struct NoopLaneErrorLog;

impl LaneErrorLog for NoopLaneErrorLog {
    fn record(
        &self,
        _lane: crate::service::kicks::Lane,
        _error: &crate::protocol::results::ApiError,
    ) {
    }
}

/// One summary line per this window (the Pacer's backoff cap).
pub const LANE_LOG_WINDOW_MS: u64 = 30_000;

type LineSink = Arc<dyn Fn(&str) + Send + Sync>;

struct Throttle {
    window_start: u64,
    suppressed: u64,
}

/// Rate-limited daemon.log writer for lane failures (root §B3 D2): the first
/// occurrence of a (scope, code) pair logs one line; later occurrences are
/// counted and one summary line per [`LANE_LOG_WINDOW_MS`] reports the count.
/// Time comes from the injected clock. The default sink is the process
/// stderr, which the elected child routes into `daemon.log`.
pub struct RateLimitedLaneLog {
    clock: Arc<dyn Clock>,
    sink: LineSink,
    state: Mutex<HashMap<(String, String), Throttle>>,
}

impl RateLimitedLaneLog {
    pub fn new(clock: Arc<dyn Clock>, sink: LineSink) -> Self {
        Self {
            clock,
            sink,
            state: Mutex::new(HashMap::new()),
        }
    }

    /// Writes lines to the process stderr (daemon.log in the elected child).
    pub fn to_stderr(clock: Arc<dyn Clock>) -> Self {
        Self::new(
            clock,
            Arc::new(|line: &str| {
                let _ = writeln!(io::stderr(), "{line}");
            }),
        )
    }

    /// A hook payload the daemon could not parse; limited like lane errors.
    pub fn record_hook_parse_failure(&self, harness: &str, detail: &str) {
        self.note(
            format!("hook {harness}"),
            "parse_failure".to_owned(),
            detail,
            0,
        );
    }

    /// Health could not read the harness version evidence; limited like lane
    /// errors (Health is read often, the failure is the same each time).
    pub fn record_harness_states_unavailable(&self, detail: &str) {
        self.note(
            "harness states".to_owned(),
            "unavailable".to_owned(),
            detail,
            0,
        );
    }

    /// Writes one line as is: for rare events that need no rate limit (a
    /// harness binary changing under the running daemon).
    pub fn write_line(&self, line: &str) {
        (self.sink)(&one_line(line));
    }

    fn note(&self, scope: String, code: String, detail: &str, attempt: u32) {
        let now = self.clock.monotonic_now().0;
        let line = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            match state.entry((scope.clone(), code.clone())) {
                Entry::Vacant(slot) => {
                    slot.insert(Throttle {
                        window_start: now,
                        suppressed: 0,
                    });
                    Some(format!("{scope}: {code}: {}", one_line(detail)))
                }
                Entry::Occupied(mut slot) => {
                    let throttle = slot.get_mut();
                    throttle.suppressed += 1;
                    if now.saturating_sub(throttle.window_start) >= LANE_LOG_WINDOW_MS {
                        let count = throttle.suppressed;
                        throttle.suppressed = 0;
                        throttle.window_start = now;
                        Some(format!(
                            "{scope}: {code} repeated {count} times in the last {}s (attempt {attempt})",
                            LANE_LOG_WINDOW_MS / 1000
                        ))
                    } else {
                        None
                    }
                }
            }
        };
        if let Some(line) = line {
            (self.sink)(&line);
        }
    }
}

/// Hook payloads the optimistically admitted recipes could not parse: counted
/// per harness for Health, and logged through the rate-limited lane logger.
pub struct HookParseFailures {
    log: Arc<RateLimitedLaneLog>,
    counts: Mutex<std::collections::BTreeMap<String, u64>>,
}

impl HookParseFailures {
    pub fn new(log: Arc<RateLimitedLaneLog>) -> Self {
        Self {
            log,
            counts: Mutex::new(std::collections::BTreeMap::new()),
        }
    }

    /// Counts one failure and logs it (rate limited per harness).
    pub fn record(&self, harness: &str, detail: &str) {
        if let Ok(mut counts) = self.counts.lock() {
            let count = counts.entry(harness.to_owned()).or_insert(0);
            *count = count.saturating_add(1);
        }
        self.log.record_hook_parse_failure(harness, detail);
    }

    /// Per-harness counts since boot, in harness-name order.
    pub fn snapshot(&self) -> Vec<(String, u64)> {
        self.counts
            .lock()
            .map(|counts| counts.iter().map(|(h, n)| (h.clone(), *n)).collect())
            .unwrap_or_default()
    }
}

fn one_line(detail: &str) -> String {
    detail
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

impl LaneErrorLog for RateLimitedLaneLog {
    fn record(
        &self,
        lane: crate::service::kicks::Lane,
        error: &crate::protocol::results::ApiError,
    ) {
        self.record_with_attempt(lane, error, 0);
    }

    fn record_with_attempt(
        &self,
        lane: crate::service::kicks::Lane,
        error: &crate::protocol::results::ApiError,
        attempt: u32,
    ) {
        self.note(
            format!("lane {}", lane.name()),
            format!("{:?}", error.code),
            &error.detail,
            attempt,
        );
    }

    fn record_wake_verification(
        &self,
        seat: &crate::protocol::ids::SeatId,
        verification: crate::scheduler::SubmissionVerification,
    ) {
        use crate::scheduler::SubmissionVerification::{NotChecked, Unsubmitted};
        let what = match verification {
            Unsubmitted => "unsubmitted after one submit-key retry",
            NotChecked => "submission not checked",
            _ => return,
        };
        self.note(
            "lane wakes".to_owned(),
            format!("verification {}", verification.as_str()),
            &format!("seat {}: wake prompt {what}", seat.as_str()),
            0,
        );
    }
}

pub const MAX_LOG_BYTES: u64 = 1024 * 1024;

/// File name of the elected daemon's active log inside the instance directory.
pub const DAEMON_LOG_FILE: &str = "daemon.log";

/// One `ensure` start attempt: the starter's pid and a random nonce. It names
/// the attempt's own startup log (root §B3 D1, amended by ht-p03.57).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartAttempt {
    pub starter_pid: u32,
    /// Eight lowercase hex digits.
    pub nonce: String,
}

impl StartAttempt {
    pub fn new() -> Self {
        Self {
            starter_pid: std::process::id(),
            nonce: uuid::Uuid::new_v4().simple().to_string()[..8].to_owned(),
        }
    }
}

impl Default for StartAttempt {
    fn default() -> Self {
        Self::new()
    }
}

/// The daemon log: `<instance>/daemon.log`. The bead's `StateDir` is
/// `InstancePaths` here, the type that knows the instance directory.
pub fn daemon_log_path(paths: &InstancePaths) -> PathBuf {
    paths.instance_dir.join(DAEMON_LOG_FILE)
}

/// Directory of per-attempt startup logs: `<instance>/logs`. "<state>" in
/// root §B3 D1 is the instance directory, consistent with `daemon.log`.
pub fn startup_logs_dir(paths: &InstancePaths) -> PathBuf {
    paths.instance_dir.join("logs")
}

/// One attempt's startup log: `<instance>/logs/startup-<pid>-<nonce>.log`.
/// The file itself is created by the starter (ht-p03.11), not here.
pub fn startup_log_path(paths: &InstancePaths, attempt: &StartAttempt) -> PathBuf {
    startup_logs_dir(paths).join(format!(
        "startup-{}-{}.log",
        attempt.starter_pid, attempt.nonce
    ))
}

/// Startup logs older than this are pruned.
pub const STARTUP_LOG_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
/// At most this many `startup-*.log` files survive a prune, the current one included.
pub const STARTUP_LOG_KEEP: usize = 8;
/// `ensure` prints at most this many bytes / lines of a startup log. The child
/// writes the file through its own descriptor, so the cap is applied by the
/// reader and by pruning, not by the writer.
pub const STARTUP_TAIL_BYTES: u64 = 8 * 1024;
pub const STARTUP_TAIL_LINES: usize = 40;

/// Creates the attempt's startup log: the logs directory (0700) if missing,
/// then the file with O_CREAT|O_EXCL at 0600, with the same private-file
/// checks as `daemon.log` (regular file, our uid, mode 0600, empty).
pub fn create_startup_log(
    paths: &InstancePaths,
    attempt: &StartAttempt,
) -> io::Result<(File, PathBuf)> {
    let dir = startup_logs_dir(paths);
    match fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = fs::symlink_metadata(&dir)?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != effective_uid()
        || metadata.permissions().mode() & 0o777 != 0o700
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unsafe startup log directory",
        ));
    }
    let path = startup_log_path(paths, attempt);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file()
        || metadata.uid() != effective_uid()
        || metadata.permissions().mode() & 0o777 != 0o600
        || metadata.len() != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unsafe startup log",
        ));
    }
    Ok((file, path))
}

/// Removes `startup-*.log` files older than [`STARTUP_LOG_MAX_AGE`] and all but
/// the newest [`STARTUP_LOG_KEEP`] (counting `current`, which is never
/// removed). Errors on single files are ignored: another starter may be
/// pruning the same directory.
pub fn prune_startup_logs(dir: &Path, current: &Path, now: SystemTime) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut others = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path == current || !name.starts_with("startup-") || !name.ends_with(".log") {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.file_type().is_file() {
            continue;
        }
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        others.push((modified, path));
    }
    others.sort_by(|a, b| b.0.cmp(&a.0));
    for (index, (modified, path)) in others.iter().enumerate() {
        let too_old = now
            .duration_since(*modified)
            .is_ok_and(|age| age > STARTUP_LOG_MAX_AGE);
        if too_old || index >= STARTUP_LOG_KEEP - 1 {
            let _ = fs::remove_file(path);
        }
    }
}

/// The last [`STARTUP_TAIL_LINES`] lines (within [`STARTUP_TAIL_BYTES`]) of
/// `bytes`, lossily decoded with control characters other than newline and
/// tab replaced, so child output cannot drive the operator's terminal.
pub fn startup_tail_text(bytes: &[u8]) -> String {
    let bytes = if bytes.len() as u64 > STARTUP_TAIL_BYTES {
        &bytes[bytes.len() - STARTUP_TAIL_BYTES as usize..]
    } else {
        bytes
    };
    let text: String = String::from_utf8_lossy(bytes)
        .chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                '?'
            } else {
                c
            }
        })
        .collect();
    let lines: Vec<&str> = text.lines().collect();
    let skip = lines.len().saturating_sub(STARTUP_TAIL_LINES);
    lines[skip..].join("\n")
}

/// Reads the tail of a startup log file.
pub fn read_startup_tail(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    if len > STARTUP_TAIL_BYTES {
        file.seek(SeekFrom::Start(len - STARTUP_TAIL_BYTES))?;
    }
    let mut bytes = Vec::new();
    file.take(STARTUP_TAIL_BYTES).read_to_end(&mut bytes)?;
    Ok(startup_tail_text(&bytes))
}

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
        self.directory.join(DAEMON_LOG_FILE)
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

#[cfg(test)]
#[path = "../../tests/daemon/operator_text_contract.rs"]
mod operator_text_contract;

#[cfg(test)]
#[path = "../../tests/daemon/lane_log.rs"]
mod lane_log;
