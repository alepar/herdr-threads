//! Read-only macOS libproc and sysctl observations for queue control. Every call that does
//! not return a complete record fails closed; nothing here signals or writes a process.
//! Layouts mirror `sys/proc_info.h` (measured against the macOS SDK; sizes asserted below).
use super::{
    capture::FileIdentity,
    rules::{Region, Vnode},
};
use std::{
    ffi::c_int,
    mem::{MaybeUninit, size_of},
    os::unix::ffi::OsStrExt,
    path::PathBuf,
    time::Instant,
};

#[repr(C)]
struct ProcFileInfo {
    fi_openflags: u32,
    fi_status: u32,
    fi_offset: i64,
    fi_type: i32,
    fi_guardflags: u32,
}
#[repr(C)]
struct VnodeFdInfoWithPath {
    pfi: ProcFileInfo,
    pvip: libc::vnode_info_path,
}
#[repr(C)]
struct ProcRegionInfo {
    pri_protection: u32,
    pri_max_protection: u32,
    pri_inheritance: u32,
    pri_flags: u32,
    pri_offset: u64,
    pri_behavior: u32,
    pri_user_wired_count: u32,
    pri_user_tag: u32,
    pri_pages_resident: u32,
    pri_pages_shared_now_private: u32,
    pri_pages_swapped_out: u32,
    pri_pages_dirtied: u32,
    pri_ref_count: u32,
    pri_shadow_depth: u32,
    pri_share_mode: u32,
    pri_private_pages_resident: u32,
    pri_shared_pages_resident: u32,
    pri_obj_id: u32,
    pri_depth: u32,
    pri_address: u64,
    pri_size: u64,
}
#[repr(C)]
struct ProcRegionWithPathInfo {
    prp_prinfo: ProcRegionInfo,
    prp_vip: libc::vnode_info_path,
}
const _: () = assert!(size_of::<ProcFileInfo>() == 24);
const _: () = assert!(size_of::<libc::vnode_info_path>() == 1176);
const _: () = assert!(size_of::<VnodeFdInfoWithPath>() == 1200);
const _: () = assert!(size_of::<ProcRegionInfo>() == 96);
const _: () = assert!(std::mem::offset_of!(ProcRegionInfo, pri_address) == 80);
const _: () = assert!(size_of::<ProcRegionWithPathInfo>() == 1272);
const _: () = assert!(size_of::<libc::proc_fdinfo>() == 8);
const PROC_PIDFDVNODEPATHINFO: c_int = 2;
const PROC_PIDREGIONPATHINFO: c_int = 8;
const VREG: c_int = 1;
/// The receiver's descriptor bound, matching the Linux /proc/<pid>/fd walk.
const FD_LIMIT: usize = 1024;
const REGION_LIMIT: usize = 8192;

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}
fn clear_errno() {
    // SAFETY: __error returns this thread's errno location.
    unsafe { *libc::__error() = 0 }
}
fn pid_arg(pid: u32) -> Option<c_int> {
    (pid != 0 && pid <= i32::MAX as u32).then_some(pid as c_int)
}
/// The fixed-size record `flavor` fills, or the errno of a short or failed answer.
fn pidinfo<T>(pid: c_int, flavor: c_int, arg: u64) -> Result<T, i32> {
    let mut out = MaybeUninit::<T>::zeroed();
    clear_errno();
    // SAFETY: the buffer is size_of::<T>() writable bytes; the kernel writes at most that.
    let n = unsafe {
        libc::proc_pidinfo(
            pid,
            flavor,
            arg,
            out.as_mut_ptr().cast(),
            size_of::<T>() as c_int,
        )
    };
    if n as usize != size_of::<T>() {
        return Err(errno());
    }
    // SAFETY: a full record was written over zeroed plain-old-data.
    Ok(unsafe { out.assume_init() })
}
fn vnode_path(vip: &libc::vnode_info_path) -> PathBuf {
    // SAFETY: vip_path is 1024 contiguous c_char bytes.
    let raw: &[u8; 1024] = unsafe { &*(vip.vip_path.as_ptr() as *const [u8; 1024]) };
    let end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
    PathBuf::from(std::ffi::OsStr::from_bytes(&raw[..end]))
}
fn vnode(vip: &libc::vnode_info_path) -> Vnode {
    let st = &vip.vip_vi.vi_stat;
    Vnode {
        path: vnode_path(vip),
        device: u64::from(st.vst_dev),
        inode: st.vst_ino,
        nlink: u64::from(st.vst_nlink),
        regular: vip.vip_vi.vi_type == VREG,
    }
}

/// Every vnode descriptor of `pid` with its own identity, or None when the table is
/// unreadable, exceeds the bound (a full buffer is silent truncation) or changes shape.
pub(super) fn open_vnodes(pid: u32, deadline: Instant) -> Option<Vec<Vnode>> {
    let pid = pid_arg(pid)?;
    let mut fds = vec![
        libc::proc_fdinfo {
            proc_fd: 0,
            proc_fdtype: 0
        };
        FD_LIMIT + 1
    ];
    let bytes = (fds.len() * size_of::<libc::proc_fdinfo>()) as c_int;
    clear_errno();
    // SAFETY: `fds` is `bytes` writable bytes; the kernel writes at most that.
    let n = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDLISTFDS,
            0,
            fds.as_mut_ptr().cast(),
            bytes,
        )
    };
    if n <= 0 || !(n as usize).is_multiple_of(size_of::<libc::proc_fdinfo>()) {
        return None;
    }
    let count = n as usize / size_of::<libc::proc_fdinfo>();
    if count > FD_LIMIT {
        return None;
    }
    let mut out = Vec::new();
    for fd in &fds[..count] {
        if Instant::now() >= deadline {
            return None;
        }
        if fd.proc_fdtype != libc::PROX_FDTYPE_VNODE as u32 {
            continue;
        }
        let mut info = MaybeUninit::<VnodeFdInfoWithPath>::zeroed();
        clear_errno();
        // SAFETY: the buffer holds one full record; the kernel writes at most its size.
        let k = unsafe {
            libc::proc_pidfdinfo(
                pid,
                fd.proc_fd,
                PROC_PIDFDVNODEPATHINFO,
                info.as_mut_ptr().cast(),
                size_of::<VnodeFdInfoWithPath>() as c_int,
            )
        };
        if k as usize == size_of::<VnodeFdInfoWithPath>() {
            // SAFETY: a full record was written over zeroed plain-old-data.
            out.push(vnode(&unsafe { info.assume_init() }.pvip));
        } else if errno() != libc::EBADF {
            // A descriptor closed between list and read is a race; anything else is unknown.
            return None;
        }
    }
    Some(out)
}

fn regions(pid: c_int, deadline: Instant) -> Option<Vec<Region>> {
    let mut out = Vec::new();
    let mut address = 0u64;
    for _ in 0..REGION_LIMIT {
        if Instant::now() >= deadline {
            return None;
        }
        match pidinfo::<ProcRegionWithPathInfo>(pid, PROC_PIDREGIONPATHINFO, address) {
            Ok(region) => {
                let info = &region.prp_prinfo;
                if region.prp_vip.vip_vi.vi_stat.vst_ino != 0 {
                    out.push(Region {
                        vnode: vnode(&region.prp_vip),
                        executable: info.pri_protection & libc::VM_PROT_EXECUTE as u32 != 0,
                    });
                }
                let next = info.pri_address.checked_add(info.pri_size)?;
                if next <= address {
                    return None;
                }
                address = next;
            }
            // The walk past the last region ends with EINVAL; any other answer is unknown.
            Err(e) if e == libc::EINVAL => return Some(out),
            Err(_) => return None,
        }
    }
    None
}
fn pidpath(pid: c_int) -> Option<PathBuf> {
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: `buf` is PROC_PIDPATHINFO_MAXSIZE writable bytes.
    let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if n <= 0 || n as usize >= buf.len() {
        return None;
    }
    buf.truncate(n as usize);
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(&buf)))
}
/// The main image identity by `rules::executable`; no private API, no task port.
pub(super) fn executable(pid: u32, deadline: Instant) -> Option<FileIdentity> {
    let pid = pid_arg(pid)?;
    let path = pidpath(pid)?;
    let before = super::capture::identity(&path);
    let walked = regions(pid, deadline);
    let after_path = pidpath(pid);
    let after = after_path.as_deref().and_then(super::capture::identity);
    let complete = walked.is_some();
    super::rules::executable(
        &path,
        before,
        &walked.unwrap_or_default(),
        complete,
        after_path.as_deref(),
        after,
    )
}

/// argv contains `--managed-daemon`, or None when argv is unreadable.
pub(super) fn managed_daemon(pid: u32) -> Option<bool> {
    let pid = pid_arg(pid)?;
    let mut argmax: c_int = 0;
    let mut len = size_of::<c_int>();
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    // SAFETY: `argmax`/`len` describe a valid c_int out-buffer.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            (&mut argmax as *mut c_int).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || argmax <= 0 || argmax > 16 << 20 {
        return None;
    }
    let mut buf = vec![0u8; argmax as usize];
    let mut len = buf.len();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    // SAFETY: `buf`/`len` describe a valid writable buffer.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || len < 4 {
        return None;
    }
    buf.truncate(len);
    let argv = crate::host::process_table::parse_procargs2(&buf);
    if argv.is_empty() {
        return None;
    }
    Some(argv.iter().any(|a| a == "--managed-daemon"))
}

/// (start, pgid, ruid) of `pid`: start is fork time in microseconds, the same value the
/// host process table fingerprints. Err carries errno (ESRCH: gone or zombie).
pub(super) fn birth(pid: u32) -> Result<(u64, u32, u32), i32> {
    let p = pid_arg(pid).ok_or(libc::ESRCH)?;
    let info = pidinfo::<libc::proc_bsdinfo>(p, libc::PROC_PIDTBSDINFO, 0)?;
    if info.pbi_pid != pid {
        return Err(libc::ESRCH);
    }
    let start = info
        .pbi_start_tvsec
        .checked_mul(1_000_000)
        .and_then(|s| s.checked_add(info.pbi_start_tvusec))
        .ok_or(libc::EINVAL)?;
    Ok((start, info.pbi_pgid, info.pbi_ruid))
}

#[cfg(all(test, feature = "test-support"))]
#[path = "../../../../tests/harness/codex_wake_macos.rs"]
mod tests;
