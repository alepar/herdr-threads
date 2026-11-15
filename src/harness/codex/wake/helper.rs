//! Bounded, read-only observations of an owned control helper. Recovery never signals it.
use super::control::error;
use crate::{
    harness::wake::{
        WakeControlContext, WakeHelperPresence, WakeHelperQuiescence, WakeHelperWitness,
    },
    protocol::{authority::LivenessFingerprint, results::ApiError},
};
use std::{fs::File, io::Read, path::Path};

fn bounded(path: &Path) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    File::open(path)?.take(16385).read_to_end(&mut bytes)?;
    if bytes.len() > 16384 {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    String::from_utf8(bytes).map_err(|_| std::io::ErrorKind::InvalidData.into())
}
fn boot() -> Option<String> {
    let raw = bounded(Path::new("/proc/sys/kernel/random/boot_id")).ok()?;
    let boot = raw.trim();
    (!boot.is_empty() && boot.len() <= 64).then(|| boot.to_owned())
}
fn birth(dir: &Path) -> std::io::Result<(u64, u32)> {
    let raw = bounded(&dir.join("stat"))?;
    let fields: Vec<_> = raw
        .get(raw.rfind(')').ok_or(std::io::ErrorKind::InvalidData)? + 1..)
        .ok_or(std::io::ErrorKind::InvalidData)?
        .split_ascii_whitespace()
        .collect();
    let parse = || Some((fields.get(19)?.parse().ok()?, fields.get(2)?.parse().ok()?));
    parse().ok_or_else(|| std::io::ErrorKind::InvalidData.into())
}
fn same_user(dir: &Path) -> bool {
    let Ok(status) = bounded(&dir.join("status")) else {
        return false;
    };
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|uid| uid.split_ascii_whitespace().next()?.parse::<u32>().ok())
        == Some(unsafe { libc::getuid() })
}
pub(super) fn witness(pid: u32) -> Result<WakeHelperWitness, ApiError> {
    let dir = std::path::PathBuf::from(format!("/proc/{pid}"));
    let (start, pgid) = birth(&dir).map_err(|_| error("queue helper birth unavailable"))?;
    if !same_user(&dir) || pgid != pid || birth(&dir).ok() != Some((start, pgid)) {
        return Err(error("queue helper ownership unavailable"));
    }
    Ok(WakeHelperWitness {
        process: LivenessFingerprint { pid, start },
        kernel_boot: boot().ok_or_else(|| error("queue helper kernel boot unavailable"))?,
        pgid,
    })
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LeaderPresence {
    Same,
    Ended,
    Unknown,
}
pub(super) fn leader_at(dir: &Path, start: u64) -> LeaderPresence {
    let first = match birth(dir) {
        Ok(value) => value,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LeaderPresence::Ended,
        Err(_) => return LeaderPresence::Unknown,
    };
    if !same_user(dir) {
        return LeaderPresence::Unknown;
    }
    if birth(dir).ok() != Some(first) {
        return LeaderPresence::Unknown;
    }
    if first.0 == start {
        LeaderPresence::Same
    } else {
        LeaderPresence::Ended
    }
}
pub(super) fn classify(leader: LeaderPresence, group: Option<bool>) -> WakeHelperPresence {
    match (leader, group) {
        (LeaderPresence::Same, Some(true)) => WakeHelperPresence::Live,
        (LeaderPresence::Ended, Some(false)) => {
            WakeHelperPresence::Quiescent(WakeHelperQuiescence::LeaderEndedAndGroupAbsent)
        }
        _ => WakeHelperPresence::Unknown,
    }
}
pub(super) fn probe(
    witness: &WakeHelperWitness,
    context: &WakeControlContext<'_>,
) -> Result<WakeHelperPresence, ApiError> {
    if context.budget.is_exhausted(context.clock) {
        return Ok(WakeHelperPresence::Unknown);
    }
    let Some(current_boot) = boot() else {
        return Ok(WakeHelperPresence::Unknown);
    };
    if witness.kernel_boot.is_empty() || witness.kernel_boot.len() > 64 {
        return Ok(WakeHelperPresence::Unknown);
    }
    if current_boot != witness.kernel_boot {
        return Ok(WakeHelperPresence::Quiescent(
            WakeHelperQuiescence::BootChanged,
        ));
    }
    if witness.process.pid == 0
        || witness.process.pid > i32::MAX as u32
        || witness.pgid == 0
        || witness.pgid > i32::MAX as u32
    {
        return Ok(WakeHelperPresence::Unknown);
    }
    let leader = leader_at(
        Path::new(&format!("/proc/{}", witness.process.pid)),
        witness.process.start,
    );
    // Signal zero only observes group existence; it never terminates or modifies a process.
    let group = if unsafe { libc::kill(-(witness.pgid as i32), 0) } == 0 {
        Some(true)
    } else if std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
        Some(false)
    } else {
        None
    };
    if context.budget.is_exhausted(context.clock) {
        return Ok(WakeHelperPresence::Unknown);
    }
    Ok(classify(leader, group))
}
