//! Bounded local process-lifetime evidence, separate from native incarnation.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::net::UnixStream;

/// Whether this platform captures the kernel peer witness: macOS through
/// `proc_pidinfo`, Linux through `SO_PEERCRED` and procfs. Elsewhere every
/// observation is unverified.
pub const PEER_WITNESS_SUPPORTED: bool = cfg!(any(target_os = "macos", target_os = "linux"));
pub const MACOS_WITNESS_PLATFORM: &str = "macos-proc-bsdinfo-v1";
pub const LINUX_WITNESS_PLATFORM: &str = "linux-procfs-v1";

/// A witness platform this build knows how to qualify.
pub fn known_witness_platform(platform: &str) -> bool {
    platform == MACOS_WITNESS_PLATFORM || platform == LINUX_WITNESS_PLATFORM
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalEndpointWitness {
    pub schema: u32,
    pub platform: String,
    pub endpoint: PathBuf,
    pub peer_uid: u32,
    pub peer_pid: u32,
    pub start_seconds: u64,
    pub start_microseconds: u64,
    pub socket: SocketIdentity,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocketIdentity {
    pub device: u64,
    pub inode: u64,
    pub birth_seconds: i64,
    pub birth_nanoseconds: i64,
    pub change_seconds: i64,
    pub change_nanoseconds: i64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureError {
    Unsupported,
    EndpointUnavailable,
    PeerUnavailable,
    ProcessUnavailable,
    ProcessDenied,
    ShortProcessInfo,
    InvalidProcessInfo,
}
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub uid: u32,
    pub start_seconds: u64,
    pub start_microseconds: u64,
}
pub trait ProcessInfoProvider: Send + Sync {
    fn process_info(&self, pid: u32) -> Result<ProcessInfo, CaptureError>;
}
pub struct KernelProcessInfo;
impl ProcessInfoProvider for KernelProcessInfo {
    fn process_info(&self, pid: u32) -> Result<ProcessInfo, CaptureError> {
        #[cfg(target_os = "macos")]
        {
            if pid == 0 || pid > i32::MAX as u32 {
                return Err(CaptureError::InvalidProcessInfo);
            }
            let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
            let size = std::mem::size_of::<libc::proc_bsdinfo>();
            // The kernel writes at most the supplied buffer size. Only a full
            // record is initialized and consumed; no partial record is evidence.
            let count = unsafe {
                libc::proc_pidinfo(
                    pid as i32,
                    libc::PROC_PIDTBSDINFO,
                    0,
                    info.as_mut_ptr().cast(),
                    size as i32,
                )
            };
            if count <= 0 {
                return Err(match std::io::Error::last_os_error().raw_os_error() {
                    Some(libc::EPERM | libc::EACCES) => CaptureError::ProcessDenied,
                    _ => CaptureError::ProcessUnavailable,
                });
            }
            if count as usize != size {
                return Err(CaptureError::ShortProcessInfo);
            }
            let info = unsafe { info.assume_init() };
            Ok(ProcessInfo {
                pid: info.pbi_pid,
                uid: info.pbi_uid,
                start_seconds: info.pbi_start_tvsec,
                start_microseconds: info.pbi_start_tvusec,
            })
        }
        #[cfg(target_os = "linux")]
        {
            linux::process_info(pid)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = pid;
            Err(CaptureError::Unsupported)
        }
    }
}
pub fn capture_peer_witness(
    stream: &UnixStream,
    endpoint: &Path,
) -> Result<LocalEndpointWitness, CaptureError> {
    capture_peer_witness_with(stream, endpoint, &KernelProcessInfo)
}
/// Capture credentials from this connected stream. The provider must return
/// only a complete process record. Failures never produce a witness.
/// Equality is a bounded same-process inference: it does not attest native
/// incarnation, retained/inherited descriptors, hostile same-user behavior or
/// unmeasured PID/start-time reuse. Socket metadata is auxiliary evidence.
pub fn capture_peer_witness_with(
    stream: &UnixStream,
    endpoint: &Path,
    provider: &dyn ProcessInfoProvider,
) -> Result<LocalEndpointWitness, CaptureError> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let socket = socket_identity(endpoint)?;
        let peer = stream
            .peer_cred()
            .map_err(|_| CaptureError::PeerUnavailable)?;
        let pid = peer
            .pid()
            .filter(|pid| *pid > 0)
            .ok_or(CaptureError::PeerUnavailable)? as u32;
        let info = provider.process_info(pid)?;
        if info.pid != pid
            || info.uid != peer.uid()
            || info.start_seconds == 0
            || info.start_seconds > i64::MAX as u64
            || info.start_microseconds >= 1_000_000
        {
            return Err(CaptureError::InvalidProcessInfo);
        }
        Ok(LocalEndpointWitness {
            schema: 1,
            platform: if cfg!(target_os = "macos") {
                MACOS_WITNESS_PLATFORM
            } else {
                LINUX_WITNESS_PLATFORM
            }
            .into(),
            endpoint: endpoint.to_path_buf(),
            peer_uid: peer.uid(),
            peer_pid: pid,
            start_seconds: info.start_seconds,
            start_microseconds: info.start_microseconds,
            socket,
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (stream, endpoint, provider);
        Err(CaptureError::Unsupported)
    }
}

pub(crate) fn socket_identity(endpoint: &Path) -> Result<SocketIdentity, CaptureError> {
    #[cfg(target_os = "macos")]
    {
        use std::os::{
            macos::fs::MetadataExt as MacMetadataExt,
            unix::fs::{FileTypeExt, MetadataExt},
        };
        if !endpoint.is_absolute() {
            return Err(CaptureError::EndpointUnavailable);
        }
        let metadata =
            std::fs::symlink_metadata(endpoint).map_err(|_| CaptureError::EndpointUnavailable)?;
        if !metadata.file_type().is_socket() {
            return Err(CaptureError::EndpointUnavailable);
        }
        #[allow(deprecated)]
        Ok(SocketIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
            birth_seconds: metadata.st_birthtime(),
            birth_nanoseconds: metadata.st_birthtime_nsec(),
            change_seconds: metadata.ctime(),
            change_nanoseconds: metadata.ctime_nsec(),
        })
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        if !endpoint.is_absolute() {
            return Err(CaptureError::EndpointUnavailable);
        }
        let metadata =
            std::fs::symlink_metadata(endpoint).map_err(|_| CaptureError::EndpointUnavailable)?;
        if !metadata.file_type().is_socket() {
            return Err(CaptureError::EndpointUnavailable);
        }
        // statx birth time where the filesystem records one; otherwise zero,
        // which is stable for that filesystem and leaves inode and ctime.
        let (birth_seconds, birth_nanoseconds) = metadata
            .created()
            .ok()
            .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or((0, 0), |at| {
                (at.as_secs() as i64, i64::from(at.subsec_nanos()))
            });
        Ok(SocketIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
            birth_seconds,
            birth_nanoseconds,
            change_seconds: metadata.ctime(),
            change_nanoseconds: metadata.ctime_nsec(),
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = endpoint;
        Err(CaptureError::Unsupported)
    }
}

/// Recheck the original connected peer after EOF. macOS may no longer expose
/// LOCAL_PEERPID after the server closes its descriptor; never replace the
/// original peer identity with credentials from a new connection.
pub(crate) fn recheck_witness(
    witness: &LocalEndpointWitness,
    provider: &dyn ProcessInfoProvider,
) -> Result<LocalEndpointWitness, CaptureError> {
    let info = provider.process_info(witness.peer_pid)?;
    if info.pid != witness.peer_pid
        || info.uid != witness.peer_uid
        || info.start_seconds == 0
        || info.start_seconds > i64::MAX as u64
        || info.start_microseconds >= 1_000_000
    {
        return Err(CaptureError::InvalidProcessInfo);
    }
    let mut after = witness.clone();
    after.socket = socket_identity(&witness.endpoint)?;
    after.start_seconds = info.start_seconds;
    after.start_microseconds = info.start_microseconds;
    Ok(after)
}

/// Linux process records from procfs. The start time is `btime` plus the
/// process's start in clock ticks since boot (tick granularity, typically
/// 10 ms). A wall-clock step moves `btime`, so a stepped clock reads as a new
/// incarnation (holds, never a false same-process match).
#[cfg(target_os = "linux")]
mod linux {
    use super::{CaptureError, ProcessInfo};

    pub(super) fn process_info(pid: u32) -> Result<ProcessInfo, CaptureError> {
        if pid == 0 || pid > i32::MAX as u32 {
            return Err(CaptureError::InvalidProcessInfo);
        }
        let dir = std::path::PathBuf::from(format!("/proc/{pid}"));
        let ticks = start_ticks(&dir)?;
        let uid = effective_uid(&std::fs::read_to_string(dir.join("status")).map_err(read_error)?)?;
        // The pid may exit and be reused between the reads above: the record
        // counts only when the start time is unchanged around the uid read.
        if start_ticks(&dir)? != ticks {
            return Err(CaptureError::ProcessUnavailable);
        }
        let boot = boot_seconds(&std::fs::read_to_string("/proc/stat").map_err(read_error)?)?;
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if hz <= 0 {
            return Err(CaptureError::InvalidProcessInfo);
        }
        let hz = hz as u64;
        Ok(ProcessInfo {
            pid,
            uid,
            start_seconds: boot
                .checked_add(ticks / hz)
                .ok_or(CaptureError::InvalidProcessInfo)?,
            start_microseconds: (ticks % hz) * 1_000_000 / hz,
        })
    }

    fn read_error(error: std::io::Error) -> CaptureError {
        match error.raw_os_error() {
            Some(libc::EPERM | libc::EACCES) => CaptureError::ProcessDenied,
            _ => CaptureError::ProcessUnavailable,
        }
    }

    fn start_ticks(dir: &std::path::Path) -> Result<u64, CaptureError> {
        parse_start_ticks(&std::fs::read_to_string(dir.join("stat")).map_err(read_error)?)
    }

    /// Field 22 of `/proc/<pid>/stat`. The command name (field 2) is
    /// parenthesized and may itself contain spaces and parentheses, so fields
    /// are counted after its last `)`.
    pub(super) fn parse_start_ticks(stat: &str) -> Result<u64, CaptureError> {
        let rest = stat
            .rfind(')')
            .map(|end| &stat[end + 1..])
            .ok_or(CaptureError::InvalidProcessInfo)?;
        rest.split_ascii_whitespace()
            .nth(19)
            .and_then(|field| field.parse().ok())
            .ok_or(CaptureError::InvalidProcessInfo)
    }

    /// The effective uid: the second value of the `Uid:` line, the identity
    /// `SO_PEERCRED` reports.
    pub(super) fn effective_uid(status: &str) -> Result<u32, CaptureError> {
        status
            .lines()
            .find_map(|line| line.strip_prefix("Uid:"))
            .and_then(|ids| ids.split_ascii_whitespace().nth(1))
            .and_then(|uid| uid.parse().ok())
            .ok_or(CaptureError::InvalidProcessInfo)
    }

    pub(super) fn boot_seconds(stat: &str) -> Result<u64, CaptureError> {
        stat.lines()
            .find_map(|line| line.strip_prefix("btime "))
            .and_then(|seconds| seconds.trim().parse().ok())
            .filter(|seconds| *seconds > 0)
            .ok_or(CaptureError::InvalidProcessInfo)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn stat_fields_count_after_the_last_parenthesis() {
            let stat =
                "42 (a) b (c)) S 1 42 42 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 1 0 98765 1000 10";
            assert_eq!(parse_start_ticks(stat), Ok(98765));
            assert_eq!(
                parse_start_ticks("42 noparen S"),
                Err(CaptureError::InvalidProcessInfo)
            );
            assert_eq!(
                parse_start_ticks("42 (x) S 1"),
                Err(CaptureError::InvalidProcessInfo)
            );
        }

        #[test]
        fn status_and_proc_stat_lines_parse_strictly() {
            assert_eq!(
                effective_uid("Name:\tx\nUid:\t1000\t1001\t1000\t1000\n"),
                Ok(1001)
            );
            assert_eq!(
                effective_uid("Name:\tx\n"),
                Err(CaptureError::InvalidProcessInfo)
            );
            assert_eq!(
                boot_seconds("cpu 1 2\nbtime 1791500000\n"),
                Ok(1_791_500_000)
            );
            assert_eq!(
                boot_seconds("btime 0\n"),
                Err(CaptureError::InvalidProcessInfo)
            );
        }

        #[test]
        fn this_process_reads_a_stable_complete_record() {
            let pid = std::process::id();
            let first = process_info(pid).unwrap();
            let again = process_info(pid).unwrap();
            assert_eq!(first.uid, unsafe { libc::geteuid() });
            assert!(first.start_seconds > 0 && first.start_microseconds < 1_000_000);
            assert_eq!(
                (first.start_seconds, first.start_microseconds),
                (again.start_seconds, again.start_microseconds)
            );
        }
    }
}
