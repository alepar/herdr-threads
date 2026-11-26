//! Native macOS libproc observations against owned processes and scratch files only.
use super::super::{capture, helper, rules};
use super::*;
use std::{
    fs,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

const SESSION: &str = "00000000-0000-0000-0000-000000000001";
/// Both tests read this process's own descriptor table, and the bound test fills it past
/// FD_LIMIT; under `cargo test` threads they would see each other's descriptors.
static OWN_FDS: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn soon() -> Instant {
    Instant::now() + Duration::from_secs(2)
}
struct Scratch(PathBuf);
impl Scratch {
    fn new(tag: &str) -> Self {
        let root = crate::test_support::scratch::temp_path_canonical(format!(
            "ht-qwake-macos-{tag}-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        Self(fs::canonicalize(&root).unwrap())
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
/// An owned, killed-and-reaped copy of /bin/sleep started from `path`.
struct Sleeper(std::process::Child);
impl Sleeper {
    fn start(path: &Path) -> Self {
        let mut command = crate::test_support::spawn::command(path);
        command
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        // leak-guard: spawn::command tags it; Drop kills and reaps the exact child.
        let child = command.spawn().unwrap();
        let sleeper = Self(child);
        let until = soon();
        // exec has completed once libproc reports the copied image.
        while pidpath(sleeper.pid() as c_int).as_deref() != Some(path) {
            assert!(Instant::now() < until, "stand-in never exec'd");
            std::thread::sleep(Duration::from_millis(5));
        }
        sleeper
    }
    fn pid(&self) -> u32 {
        self.0.id()
    }
}
impl Drop for Sleeper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn copy_sleep(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    fs::copy("/bin/sleep", &path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    // Gatekeeper (syspolicyd) terminates a relocated copy of a platform binary shortly
    // after exec, and an ad-hoc signed one whose name changes before its asynchronous
    // exec assessment ends. An ad-hoc signature (fat layout kept) plus one completed
    // warm-up exec settles the assessment before a test renames the running image.
    let signed = std::process::Command::new("/usr/bin/codesign")
        .args(["-f", "-s", "-"])
        .arg(&path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(signed.success(), "ad-hoc signing the stand-in failed");
    // leak-guard: spawn::command tags it; status() waits for the exact child.
    let warm = crate::test_support::spawn::command(&path)
        .arg("0")
        .status()
        .unwrap();
    assert!(warm.success(), "stand-in warm-up exec failed: {warm:?}");
    path
}

#[test]
fn executable_identity_follows_the_mapped_image_and_refuses_replacement() {
    let s = Scratch::new("exe");
    let path = copy_sleep(&s.0, "standin");
    let sleeper = Sleeper::start(&path);
    assert_eq!(
        executable(sleeper.pid(), soon()),
        capture::identity(&path),
        "a fat /bin/sleep copy selects despite its non-zero slice offset"
    );
    // Rename after exec: the kernel name follows; the image is still live.
    let moved = s.0.join("moved");
    fs::rename(&path, &moved).unwrap();
    assert_eq!(executable(sleeper.pid(), soon()), capture::identity(&moved));
    // Replace after exec: a new file over the name leaves the old inode mapped.
    let fresh = copy_sleep(&s.0, "fresh");
    fs::rename(&fresh, &moved).unwrap();
    assert_eq!(executable(sleeper.pid(), soon()), None);
    drop(sleeper);
    // Unlink after exec: no process path at all.
    let path = copy_sleep(&s.0, "unlinked");
    let sleeper = Sleeper::start(&path);
    fs::remove_file(&path).unwrap();
    assert_eq!(executable(sleeper.pid(), soon()), None);
}

#[test]
fn open_vnodes_report_own_identity_and_fail_closed_past_the_bound() {
    let _fds = OWN_FDS.lock().unwrap_or_else(|e| e.into_inner());
    let s = Scratch::new("fds");
    let file = s.0.join("probe.sqlite");
    fs::write(&file, b"x").unwrap();
    let held = fs::File::open(&file).unwrap();
    let me = std::process::id();
    let seen = open_vnodes(me, soon()).unwrap();
    let id = capture::identity(&file).unwrap();
    let v = seen
        .iter()
        .find(|v| v.path == file)
        .expect("held file listed");
    assert_eq!((v.device, v.inode, v.nlink), (id.device, id.inode, 1));
    assert!(v.regular);
    // Silent truncation at a full buffer must refuse, never return a partial view.
    let many: Vec<_> = (0..FD_LIMIT + 8)
        .map(|_| fs::File::open(&file).unwrap())
        .collect();
    assert!(open_vnodes(me, soon()).is_none());
    drop(many);
    assert!(open_vnodes(me, soon()).is_some());
    drop(held);
}

#[test]
fn receiver_capture_and_hardlink_ambiguity_on_live_descriptors() {
    let _fds = OWN_FDS.lock().unwrap_or_else(|e| e.into_inner());
    let s = Scratch::new("receiver");
    fs::create_dir_all(s.0.join("sqlite")).unwrap();
    fs::create_dir_all(s.0.join("thread-writer-locks")).unwrap();
    let db = s.0.join("sqlite/queue_1.sqlite");
    let lock = s.0.join(format!("thread-writer-locks/{SESSION}.lock"));
    fs::write(&db, b"x").unwrap();
    fs::write(&lock, b"").unwrap();
    let (_db, _lock) = (fs::File::open(&db).unwrap(), fs::File::open(&lock).unwrap());
    let me = std::process::id();
    let files = open_vnodes(me, soon()).unwrap();
    let (database, session_lock) =
        rules::receiver_files(&files, SESSION, &capture::identity).unwrap();
    assert_eq!(Some(database), capture::identity(&db));
    assert_eq!(Some(session_lock), capture::identity(&lock));
    // A second name for the database, then removal of the opened name.
    let link = s.0.join("sqlite/queue_1.sqlite.link");
    fs::hard_link(&db, &link).unwrap();
    assert!(
        rules::receiver_files(
            &open_vnodes(me, soon()).unwrap(),
            SESSION,
            &capture::identity
        )
        .is_none(),
        "a hardlinked database is ambiguous"
    );
    fs::remove_file(&db).unwrap();
    assert!(
        rules::receiver_files(
            &open_vnodes(me, soon()).unwrap(),
            SESSION,
            &capture::identity
        )
        .is_none(),
        "the opened name is gone"
    );
}

#[test]
fn argv_and_birth_read_owned_process_fields() {
    let s = Scratch::new("birth");
    let sleeper = Sleeper::start(&copy_sleep(&s.0, "standin"));
    assert_eq!(managed_daemon(sleeper.pid()), Some(false));
    let (start, pgid, ruid) = birth(sleeper.pid()).unwrap();
    assert_eq!(pgid, sleeper.pid());
    assert_eq!(ruid, unsafe { libc::getuid() });
    let witness = helper::witness(sleeper.pid()).unwrap();
    assert_eq!(witness.process.start, start);
    assert!(
        witness
            .process
            .is_alive(&crate::host::process_table::HostProcessTable),
        "helper start matches the host process table fingerprint"
    );
    assert_eq!(birth(0), Err(libc::ESRCH));
    // pid 1 belongs to root: readable answers fail closed.
    assert!(helper::witness(1).is_err());
}
