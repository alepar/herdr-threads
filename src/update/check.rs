//! The update check: `<node_dir>/update-check.json`, the daemon's daily check and the
//! CLI's update notice.
//!
//! - The daemon of an installer-managed installation checks the releases API once a day
//!   ([`spawn_daemon_timer`]): due [`INTERVAL`] after the last persisted attempt, run
//!   [`START_DELAY`] after start when already due, so a daemon that idle-exits still checks
//!   daily. `update --check` runs the same [`run_check`] in the foreground.
//! - The state is a node-local advisory file, never the store: a lost or corrupt file means
//!   "unknown". Writers (the check, the notice's per-caller timestamps) replace it by atomic
//!   rename; last writer wins, at worst one extra notice.
//! - The notice ([`notice_line`]) shows while `latest` is newer than the reading executable,
//!   so an update silences it with no write. `settings.json` `"update_check": "off"` or
//!   `HERDR_THREADS_OFFLINE=1` turns off both the check and the notice.
use super::{
    install::Installation,
    release::{self, Version},
};
use crate::daemon::settings::{InstanceSettings, UpdateCheckSetting};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

pub const STATE_FILE: &str = "update-check.json";
/// Checks and notices are at most this often (per node, per caller).
pub const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// The daemon's delay before an overdue check.
pub const START_DELAY: Duration = Duration::from_secs(30);
/// The caller key of a caller without a session or pane.
pub const NO_CALLER: &str = "-";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CheckState {
    pub attempted_at_ms: Option<i64>,
    pub checked_at_ms: Option<i64>,
    /// The newest published release at the last successful check (`X.Y.Z`).
    pub latest: Option<String>,
    /// Caller key -> when that caller was last shown the notice.
    pub notified: BTreeMap<String, i64>,
}

pub fn state_path(node_dir: &Path) -> PathBuf {
    node_dir.join(STATE_FILE)
}

/// The state, or the default when the file is missing, unreadable or invalid.
pub fn read_state(node_dir: &Path) -> CheckState {
    std::fs::read(state_path(node_dir))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Replace the state file (mode 0600, atomic rename). Only into an existing directory.
pub fn write_state(node_dir: &Path, state: &CheckState) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let bytes = serde_json::to_vec(state).map_err(std::io::Error::other)?;
    let tmp = node_dir.join(format!(
        ".{STATE_FILE}.{}.{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, state_path(node_dir))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn interval_ms() -> i64 {
    i64::try_from(INTERVAL.as_millis()).unwrap_or(i64::MAX)
}

/// Whether the check and the notice are on: the setting and the offline switch.
pub fn enabled(settings: &InstanceSettings, offline_env: Option<&OsStr>) -> bool {
    settings.update_check == UpdateCheckSetting::Auto
        && !offline_env.is_some_and(|value| value == "1")
}

/// When the next daemon check is due (epoch ms): [`INTERVAL`] after the last attempt.
pub fn due_at_ms(state: &CheckState) -> i64 {
    state
        .attempted_at_ms
        .map_or(i64::MIN, |at| at.saturating_add(interval_ms()))
}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Newer(Version),
    UpToDate(Version),
}

/// Fetch the latest release, record the attempt and, on success, `latest`. `installed` is the
/// version compared against.
pub fn run_check(
    node_dir: &Path,
    installed: Version,
    now_ms: i64,
    timeout_secs: u64,
) -> Result<Outcome, String> {
    let mut state = read_state(node_dir);
    state.attempted_at_ms = Some(now_ms);
    let result = release::fetch_release(&release::api_base(), None, node_dir, timeout_secs);
    if let Ok(found) = &result {
        state.checked_at_ms = Some(now_ms);
        state.latest = Some(found.version.to_string());
    }
    let _ = write_state(node_dir, &state);
    let latest = result.map_err(|error| error.to_string())?.version;
    Ok(if latest > installed {
        Outcome::Newer(latest)
    } else {
        Outcome::UpToDate(installed)
    })
}

/// Record `latest` after an update (and its attempt), keeping the notice timestamps.
pub fn record_latest(node_dir: &Path, latest: Version, now_ms: i64) {
    let mut state = read_state(node_dir);
    state.attempted_at_ms = Some(now_ms);
    state.checked_at_ms = Some(now_ms);
    state.latest = Some(latest.to_string());
    let _ = write_state(node_dir, &state);
}

/// The notice for `caller`, when one is due: `latest` is newer than `own` and this caller
/// has not been shown it within [`INTERVAL`]. Records the showing (pruning entries older than
/// the interval) before returning the line.
pub fn notice_line(node_dir: &Path, own: Version, caller: &str, now_ms: i64) -> Option<String> {
    let mut state = read_state(node_dir);
    let latest = Version::parse(state.latest.as_deref()?)?;
    if latest <= own {
        return None;
    }
    let horizon = now_ms.saturating_sub(interval_ms());
    if state.notified.get(caller).is_some_and(|at| *at > horizon) {
        return None;
    }
    state.notified.retain(|_, at| *at > horizon);
    state.notified.insert(caller.to_owned(), now_ms);
    write_state(node_dir, &state).ok()?;
    Some(format!(
        "[herdr-threads] {latest} available: herdr-threads update"
    ))
}

/// The daemon's daily check for `node_dir`, on a detached thread, when this daemon's
/// executable is installer-managed (the caller checks [`enabled`]). `log` gets one line per
/// outcome.
pub fn spawn_daemon_timer(
    node_dir: PathBuf,
    now_ms: impl Fn() -> i64 + Send + 'static,
    log: impl Fn(&str) + Send + 'static,
) {
    let Some(installation) = Installation::current() else {
        return;
    };
    let installed = installation
        .marker
        .version
        .as_deref()
        .and_then(Version::parse)
        .unwrap_or_else(Version::current);
    let spawned = std::thread::Builder::new()
        .name("update-check".into())
        .spawn(move || {
            loop {
                let due = due_at_ms(&read_state(&node_dir));
                let wait_ms = due.saturating_sub(now_ms()).max(0);
                let wait =
                    Duration::from_millis(u64::try_from(wait_ms).unwrap_or(0)).max(START_DELAY);
                std::thread::sleep(wait);
                // Another daemon run or `update --check` may have checked meanwhile.
                if due_at_ms(&read_state(&node_dir)) > now_ms() {
                    continue;
                }
                match run_check(&node_dir, installed, now_ms(), release::API_TIMEOUT_SECS) {
                    Ok(Outcome::Newer(latest)) => log(&format!("update check: {latest} available")),
                    Ok(Outcome::UpToDate(_)) => {}
                    Err(error) => log(&format!("update check failed: {error}")),
                }
            }
        });
    if let Err(error) = spawned {
        eprintln!("herdr-threads: update check not started: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);
    impl Dir {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    fn node() -> Dir {
        let dir = crate::test_support::scratch::temp_path(format!(
            "update-check-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }

    #[test]
    fn notice_shows_once_per_caller_per_interval_and_only_for_a_newer_release() {
        let dir = node();
        let own = Version(2, 7, 0);
        assert_eq!(notice_line(dir.path(), own, "a", 1_000), None, "no state");
        write_state(
            dir.path(),
            &CheckState {
                latest: Some("2.7.1".into()),
                ..CheckState::default()
            },
        )
        .unwrap();
        assert_eq!(
            notice_line(dir.path(), own, "a", 1_000).as_deref(),
            Some("[herdr-threads] 2.7.1 available: herdr-threads update")
        );
        assert_eq!(
            notice_line(dir.path(), own, "a", 2_000),
            None,
            "same caller"
        );
        assert!(
            notice_line(dir.path(), own, "b", 2_000).is_some(),
            "another caller"
        );
        let later = 2_000 + interval_ms() + 1;
        assert!(
            notice_line(dir.path(), own, "a", later).is_some(),
            "next day"
        );
        assert!(
            !read_state(dir.path()).notified.contains_key("b"),
            "entries older than the interval are pruned"
        );
        assert_eq!(
            notice_line(dir.path(), Version(2, 7, 1), "c", later),
            None,
            "updated"
        );
    }

    #[test]
    fn notice_line_size_is_as_stated() {
        let dir = node();
        write_state(
            dir.path(),
            &CheckState {
                latest: Some("2.7.1".into()),
                ..CheckState::default()
            },
        )
        .unwrap();
        let line = notice_line(dir.path(), Version(2, 7, 0), "a", 1).unwrap();
        assert_eq!(line.len(), 53);
    }

    #[test]
    fn corrupt_state_reads_as_unknown() {
        let dir = node();
        std::fs::write(state_path(dir.path()), b"{not json").unwrap();
        assert_eq!(read_state(dir.path()), CheckState::default());
        assert_eq!(notice_line(dir.path(), Version(1, 0, 0), "a", 1), None);
    }

    #[test]
    fn due_is_an_interval_after_the_last_attempt() {
        assert_eq!(due_at_ms(&CheckState::default()), i64::MIN);
        let state = CheckState {
            attempted_at_ms: Some(5),
            ..CheckState::default()
        };
        assert_eq!(due_at_ms(&state), 5 + interval_ms());
    }

    #[test]
    fn the_setting_and_the_offline_switch_turn_it_off() {
        let mut settings = InstanceSettings::default();
        assert!(enabled(&settings, None));
        assert!(enabled(&settings, Some(OsStr::new("0"))));
        assert!(enabled(&settings, Some(OsStr::new(""))));
        assert!(!enabled(&settings, Some(OsStr::new("1"))));
        settings.update_check = UpdateCheckSetting::Off;
        assert!(!enabled(&settings, None));
    }

    #[test]
    fn state_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = node();
        write_state(dir.path(), &CheckState::default()).unwrap();
        let mode = std::fs::metadata(state_path(dir.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
