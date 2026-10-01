//! Bounded local process-lifetime evidence, separate from native incarnation.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::net::UnixStream;

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
        #[cfg(not(target_os = "macos"))]
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
    #[cfg(target_os = "macos")]
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
            platform: "macos-proc-bsdinfo-v1".into(),
            endpoint: endpoint.to_path_buf(),
            peer_uid: peer.uid(),
            peer_pid: pid,
            start_seconds: info.start_seconds,
            start_microseconds: info.start_microseconds,
            socket,
        })
    }
    #[cfg(not(target_os = "macos"))]
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
    #[cfg(not(target_os = "macos"))]
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
