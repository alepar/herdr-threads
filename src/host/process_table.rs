//! The real `ports::ProcessInfo`: parent, start stamp and argv of any same-user process.
//!
//! The start stamp is opaque and only compared with another stamp from this same reader on this
//! host (see `LivenessFingerprint`). macOS: microseconds since the Unix epoch from
//! `proc_pidinfo(PROC_PIDTBSDINFO)`. Linux: `/proc/<pid>/stat` field 22 (clock ticks since boot).
use crate::ports::{ProcessInfo, ProcessRecord};

pub struct HostProcessTable;

impl ProcessInfo for HostProcessTable {
    fn process(&self, pid: u32) -> Option<ProcessRecord> {
        read_process(pid)
    }
}

/// The argv of a `KERN_PROCARGS2` buffer: `i32 argc`, the exec path, NUL padding, then `argc`
/// NUL-terminated arguments. A buffer holding fewer than `argc` arguments yields none.
pub(crate) fn parse_procargs2(buf: &[u8]) -> Vec<String> {
    let Some(head) = buf.get(..4) else {
        return Vec::new();
    };
    let argc = i32::from_ne_bytes([head[0], head[1], head[2], head[3]]);
    let Ok(argc) = usize::try_from(argc) else {
        return Vec::new();
    };
    let rest = &buf[4..];
    let Some(path_end) = rest.iter().position(|b| *b == 0) else {
        return Vec::new();
    };
    let mut args = &rest[path_end..];
    while let Some((0, tail)) = args.split_first().map(|(b, t)| (*b, t)) {
        args = tail;
    }
    let mut argv = Vec::new();
    for _ in 0..argc {
        let Some(end) = args.iter().position(|b| *b == 0) else {
            return Vec::new();
        };
        argv.push(String::from_utf8_lossy(&args[..end]).into_owned());
        args = &args[end + 1..];
    }
    argv
}

#[cfg(target_os = "macos")]
fn read_process(pid: u32) -> Option<ProcessRecord> {
    if pid == 0 || pid > i32::MAX as u32 {
        return None;
    }
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    // The kernel writes at most the supplied buffer size; only a full record is consumed.
    let count = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size as i32,
        )
    };
    if count as usize != size {
        return None;
    }
    let info = unsafe { info.assume_init() };
    if info.pbi_pid != pid {
        return None;
    }
    let start = info
        .pbi_start_tvsec
        .checked_mul(1_000_000)?
        .checked_add(info.pbi_start_tvusec)?;
    Some(ProcessRecord {
        pid,
        parent: Some(info.pbi_ppid),
        start,
        argv: macos_argv(pid),
    })
}

/// Unreadable argv (another user's process, a race) is an empty vector, not an error.
#[cfg(target_os = "macos")]
fn macos_argv(pid: u32) -> Vec<String> {
    let mut argmax: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>();
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    // SAFETY: `argmax`/`len` describe a valid c_int out-buffer.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            (&mut argmax as *mut libc::c_int).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || argmax <= 0 {
        return Vec::new();
    }
    let mut buf = vec![0u8; argmax as usize];
    let mut len = buf.len();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
    // SAFETY: `buf`/`len` describe a valid writable buffer; the kernel writes at most `len` bytes.
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
    if rc != 0 {
        return Vec::new();
    }
    buf.truncate(len);
    parse_procargs2(&buf)
}

#[cfg(target_os = "linux")]
fn read_process(pid: u32) -> Option<ProcessRecord> {
    use crate::host::continuity::parse_proc_stat;
    if pid == 0 || pid > i32::MAX as u32 {
        return None;
    }
    let dir = std::path::PathBuf::from(format!("/proc/{pid}"));
    let stat = |dir: &std::path::Path| {
        parse_proc_stat(&std::fs::read_to_string(dir.join("stat")).ok()?).ok()
    };
    let (parent, start) = stat(&dir)?;
    let cmdline = std::fs::read(dir.join("cmdline")).unwrap_or_default();
    let argv = cmdline
        .split(|b| *b == 0)
        .filter(|arg| !arg.is_empty())
        .map(|arg| String::from_utf8_lossy(arg).into_owned())
        .collect();
    // The pid may exit and be reused between the reads: count the record only when the start
    // stamp is unchanged around the argv read (same rule as continuity).
    if stat(&dir)?.1 != start {
        return None;
    }
    Some(ProcessRecord {
        pid,
        parent: Some(parent),
        start,
        argv,
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn read_process(_pid: u32) -> Option<ProcessRecord> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::continuity::parse_proc_stat;

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn this_process_reads_itself() {
        let pid = std::process::id();
        let first = HostProcessTable.process(pid).expect("own record");
        assert_eq!(first.pid, pid);
        assert_eq!(first.parent, Some(unsafe { libc::getppid() } as u32));
        assert!(!first.argv.is_empty());
        assert!(first.start > 0);
        let second = HostProcessTable.process(pid).unwrap();
        assert_eq!(first.start, second.start);
        assert!(HostProcessTable.process(0).is_none());
    }

    #[test]
    fn parse_procargs2_reads_argv() {
        let mut buf = 3i32.to_ne_bytes().to_vec();
        buf.extend_from_slice(b"/usr/bin/tool\0\0\0\0");
        buf.extend_from_slice(b"a\0b c\0d\0");
        buf.extend_from_slice(b"ENV=1\0");
        assert_eq!(parse_procargs2(&buf), ["a", "b c", "d"]);
        // Fewer arguments than argc, a missing path terminator and a short header all give none.
        let mut short = 3i32.to_ne_bytes().to_vec();
        short.extend_from_slice(b"/bin/x\0\0a\0b\0");
        assert!(parse_procargs2(&short).is_empty());
        let mut no_nul = 1i32.to_ne_bytes().to_vec();
        no_nul.extend_from_slice(b"/bin/x");
        assert!(parse_procargs2(&no_nul).is_empty());
        assert!(parse_procargs2(&[1, 0]).is_empty());
        assert!(parse_procargs2(&(-1i32).to_ne_bytes()).is_empty());
    }

    #[test]
    fn parse_proc_stat_reads_ppid_and_start() {
        let stat = "42 (a) b (c)) S 7 42 42 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 1 0 98765 1000 10";
        assert_eq!(parse_proc_stat(stat).unwrap(), (7, 98765));
        assert!(parse_proc_stat("42 (x) S 7").is_err());
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn a_renamed_parent_shows_its_argv0() {
        use crate::test_support::spawn::SpawnOwned;
        use std::io::{BufRead, BufReader};
        use std::process::Stdio;
        let mut command = crate::test_support::harness_shim::under_harness(
            "claude",
            std::path::Path::new("/bin/sh"),
            &["-c", "echo $$; exec sleep 30"],
            &[],
        );
        let mut child = command
            .stdout(Stdio::piped())
            .spawn_owned()
            .expect("spawn shim");
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let payload: u32 = line.trim().parse().expect("payload pid");
        let record = HostProcessTable.process(payload).expect("payload record");
        let parent = HostProcessTable
            .process(record.parent.expect("payload has a parent"))
            .expect("parent record");
        let _ = child.stop();
        assert_eq!(parent.pid, child.id());
        assert_eq!(parent.argv[0], "claude");
        assert_eq!(
            crate::harness::caller_env::classify(&parent, None),
            Some("claude")
        );
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn a_managed_daemon_parent_is_recognized() {
        use crate::test_support::spawn::SpawnOwned;
        use std::io::{BufRead, BufReader};
        use std::process::Stdio;
        let mut command = crate::test_support::harness_shim::under_codex_managed_daemon(
            std::path::Path::new("/bin/sh"),
            &["-c", "echo $$; exec sleep 30"],
            &[],
        );
        let mut child = command
            .stdout(Stdio::piped())
            .spawn_owned()
            .expect("spawn shim");
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let payload: u32 = line.trim().parse().expect("payload pid");
        let parent = HostProcessTable
            .process(HostProcessTable.process(payload).unwrap().parent.unwrap())
            .unwrap();
        let _ = child.stop();
        assert!(crate::harness::caller_env::is_managed_daemon(&parent));
        let plain = ProcessRecord {
            argv: vec!["codex".into()],
            ..parent
        };
        assert!(!crate::harness::caller_env::is_managed_daemon(&plain));
    }
}
