//! The real `ports::ProcessInfo`: parent, start stamp and argv of a process this user started.
//!
//! The reader returns only processes whose real uid is this process's real uid, checked before
//! the argv is read (a pid whose owner cannot be read is skipped). `/proc/<pid>/cmdline` is
//! world-readable on Linux, so without this another local user's `herdr --remote` client would
//! be visible to discovery.
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
/// Only the macOS reader calls it; tests exercise it on every platform.
#[cfg(any(target_os = "macos", test))]
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

/// Where a scan reads pids from. The owner is read before the record, so a pid another user
/// owns is skipped without its argv ever being read.
pub(crate) trait ProcessSource {
    fn pids(&self) -> Vec<u32>;
    /// The pid's real uid; `None` when unreadable (the pid is then skipped).
    fn owner(&self, pid: u32) -> Option<u32>;
    /// The full record (argv included). Only called for this user's pids.
    fn record(&self, pid: u32) -> Option<ProcessRecord>;
}

/// True when a record owned by `record_uid` belongs to this user (`my_uid`).
pub(crate) fn same_user(record_uid: u32, my_uid: u32) -> bool {
    record_uid == my_uid
}

/// This process's real uid. Platforms without a process reader never match a real uid.
#[cfg(unix)]
pub(crate) fn my_uid() -> u32 {
    // SAFETY: `getuid` takes no arguments, cannot fail and touches no memory.
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
pub(crate) fn my_uid() -> u32 {
    u32::MAX
}

/// The records of `source` owned by `my_uid`; every other pid is skipped before its record is read.
pub(crate) fn scan_from(source: &dyn ProcessSource, my_uid: u32) -> Vec<ProcessRecord> {
    source
        .pids()
        .into_iter()
        .filter(|pid| source.owner(*pid).is_some_and(|uid| same_user(uid, my_uid)))
        .filter_map(|pid| source.record(pid))
        .collect()
}

/// The host's own process table.
pub(crate) struct HostSource;

impl ProcessSource for HostSource {
    fn pids(&self) -> Vec<u32> {
        list_pids()
    }
    fn owner(&self, pid: u32) -> Option<u32> {
        host_owner(pid)
    }
    fn record(&self, pid: u32) -> Option<ProcessRecord> {
        read_process(pid)
    }
}

/// Every process this user started that this reader can describe, one record per pid. Other
/// users' processes are skipped before their argv is read. The scan is racy by nature (a process
/// may exit between the listing and its read); such a pid is simply absent.
pub fn scan() -> Vec<ProcessRecord> {
    scan_from(&HostSource, my_uid())
}

/// The first (real) uid of a `/proc/<pid>/status` `Uid:` line, `None` when missing or malformed.
/// Only the Linux reader calls it; tests exercise it on every platform.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn parse_status_real_uid(status: &str) -> Option<u32> {
    let line = status.lines().find_map(|line| line.strip_prefix("Uid:"))?;
    line.split_whitespace().next()?.parse().ok()
}

#[cfg(target_os = "linux")]
fn host_owner(pid: u32) -> Option<u32> {
    parse_status_real_uid(&std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?)
}

#[cfg(target_os = "macos")]
fn host_owner(pid: u32) -> Option<u32> {
    macos_bsdinfo(pid).map(|info| info.pbi_ruid)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn host_owner(_pid: u32) -> Option<u32> {
    None
}

/// The `<target>` of every `herdr --remote <target>` (or `--remote=<target>`) client in `rows`,
/// first-seen order, without duplicates. The executable is matched by the base name of its argv[0]
/// (`herdr`), so a renamed or wrapped binary is not seen: discovery is best effort.
pub fn remote_targets(rows: &[ProcessRecord]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for row in rows {
        let Some(program) = row.argv.first() else {
            continue;
        };
        if program.rsplit('/').next() != Some("herdr") {
            continue;
        }
        let mut args = row.argv.iter().skip(1);
        while let Some(arg) = args.next() {
            let target = if arg == "--remote" {
                args.next().map(String::as_str)
            } else {
                arg.strip_prefix("--remote=")
            };
            if let Some(target) = target.filter(|target| !target.is_empty())
                && !found.iter().any(|known| known == target)
            {
                found.push(target.to_owned());
            }
        }
    }
    found
}

#[cfg(target_os = "macos")]
fn list_pids() -> Vec<u32> {
    // SAFETY: a null buffer asks only for the byte count of the pid list.
    let bytes = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    let Ok(count) = usize::try_from(bytes) else {
        return Vec::new();
    };
    // Room for processes that start between the two calls.
    let mut pids = vec![0 as libc::pid_t; count + 64];
    let size = (pids.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
    // SAFETY: `pids` is a writable buffer of `size` bytes; the kernel writes at most that many.
    let written = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), size) };
    let Ok(written) = usize::try_from(written) else {
        return Vec::new();
    };
    pids.truncate(written);
    pids.into_iter()
        .filter_map(|pid| u32::try_from(pid).ok())
        .collect()
}

#[cfg(target_os = "linux")]
fn list_pids() -> Vec<u32> {
    std::fs::read_dir("/proc")
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn list_pids() -> Vec<u32> {
    Vec::new()
}

#[cfg(target_os = "macos")]
fn macos_bsdinfo(pid: u32) -> Option<libc::proc_bsdinfo> {
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
    Some(info)
}

#[cfg(target_os = "macos")]
fn read_process(pid: u32) -> Option<ProcessRecord> {
    let info = macos_bsdinfo(pid)?;
    // Another user's process is never read, argv included.
    if !same_user(info.pbi_ruid, my_uid()) {
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
    // Fail closed, and before `cmdline` (world-readable) is touched: only this user's pids.
    if !host_owner(pid).is_some_and(|uid| same_user(uid, my_uid())) {
        return None;
    }
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
    fn same_user_compares_record_and_own_uid() {
        assert!(same_user(501, 501));
        assert!(!same_user(0, 501));
        assert!(!same_user(1001, 1000));
    }

    #[test]
    fn parse_status_real_uid_reads_first_uid_field() {
        let status = "Name:\tx\nUmask:\t0022\nUid:\t1000\t0\t0\t0\nGid:\t1000\t1000\t1000\t1000\n";
        assert_eq!(parse_status_real_uid(status), Some(1000));
        assert_eq!(parse_status_real_uid("Name:\tx\n"), None);
        assert_eq!(parse_status_real_uid("Uid:\t\n"), None);
        assert_eq!(parse_status_real_uid("Uid:\tabc\t1\t1\t1\n"), None);
    }

    struct FakeSource(std::sync::Mutex<Vec<u32>>);

    impl ProcessSource for FakeSource {
        fn pids(&self) -> Vec<u32> {
            vec![10, 11, 12]
        }
        fn owner(&self, pid: u32) -> Option<u32> {
            match pid {
                10 => Some(501),
                11 => Some(777),
                _ => None,
            }
        }
        fn record(&self, pid: u32) -> Option<ProcessRecord> {
            self.0.lock().unwrap().push(pid);
            Some(ProcessRecord {
                pid,
                parent: None,
                start: 1,
                argv: vec!["herdr".into(), "--remote".into(), pid.to_string()],
            })
        }
    }

    #[test]
    fn scan_from_skips_foreign_pids_before_reading_them() {
        let fake = FakeSource(std::sync::Mutex::new(Vec::new()));
        let rows = scan_from(&fake, 501);
        assert_eq!(rows.iter().map(|r| r.pid).collect::<Vec<_>>(), [10]);
        assert_eq!(*fake.0.lock().unwrap(), [10], "11 and 12 never read");
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn a_foreign_owner_is_not_read() {
        // pid 1 (launchd/init) is root-owned.
        if my_uid() != 0 {
            assert!(HostProcessTable.process(1).is_none());
            assert!(!scan().iter().any(|row| row.pid == 1));
        }
    }

    fn row(argv: &[&str]) -> ProcessRecord {
        ProcessRecord {
            pid: 1,
            parent: None,
            start: 1,
            argv: argv.iter().map(|arg| (*arg).to_owned()).collect(),
        }
    }

    #[test]
    fn process_table_remote_targets() {
        let rows = [
            row(&["herdr", "--remote", "alice@box"]),
            row(&[
                "/home/u/.local/bin/herdr",
                "--remote=other",
                "--session",
                "x",
            ]),
            // Duplicate, wrong program, no target, empty target, a flag that merely ends so.
            row(&["herdr", "--remote", "alice@box"]),
            row(&["ssh", "--remote", "not-herdr"]),
            row(&["herdr", "--remote"]),
            row(&["herdr", "--remote="]),
            row(&["herdr", "--not-remote", "x"]),
            row(&[]),
        ];
        assert_eq!(remote_targets(&rows), ["alice@box", "other"]);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn scan_lists_this_process() {
        let pid = std::process::id();
        assert!(scan().iter().any(|row| row.pid == pid));
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
