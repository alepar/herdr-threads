//! macOS libproc selection rules, exercised on every platform through synthetic views.
use super::*;
use std::path::{Path, PathBuf};

const SESSION: &str = "00000000-0000-0000-0000-000000000001";
fn id(path: &str, device: u64, inode: u64) -> FileIdentity {
    FileIdentity {
        path: PathBuf::from(path),
        device,
        inode,
    }
}
fn vnode(path: &str, device: u64, inode: u64) -> Vnode {
    Vnode {
        path: PathBuf::from(path),
        device,
        inode,
        nlink: 1,
        regular: true,
    }
}
const DB: &str = "/private/tmp/h/sqlite/queue_1.sqlite";
const LOCK: &str = "/private/tmp/h/thread-writer-locks/00000000-0000-0000-0000-000000000001.lock";
/// The filesystem as `stat` currently sees it.
fn fs(entries: &[FileIdentity]) -> impl Fn(&Path) -> Option<FileIdentity> + '_ {
    move |p| entries.iter().find(|e| e.path == p).cloned()
}
fn receiver() -> (Vec<Vnode>, Vec<FileIdentity>) {
    (
        vec![
            vnode("/dev/ttys001", 855623173, 3),
            vnode(DB, 16777233, 10),
            vnode("/private/tmp/h/sqlite/queue_1.sqlite-wal", 16777233, 11),
            vnode(LOCK, 16777233, 12),
            vnode("/private/tmp/h/.tmp/plugins.sync.lock", 16777233, 13),
        ],
        vec![id(DB, 16777233, 10), id(LOCK, 16777233, 12)],
    )
}

#[test]
fn receiver_files_select_the_one_live_database_and_session_lock() {
    let (files, now) = receiver();
    let (db, lock) = receiver_files(&files, SESSION, &fs(&now)).unwrap();
    assert_eq!(db, id(DB, 16777233, 10));
    assert_eq!(lock, id(LOCK, 16777233, 12));
    assert!(receiver_files(&files, "not-a-uuid", &fs(&now)).is_none());
    assert!(
        receiver_files(&files, &format!("{{{SESSION}}}"), &fs(&now)).is_none(),
        "non-canonical session spelling"
    );
}

#[test]
fn receiver_files_refuse_every_name_ambiguity_instead_of_skipping() {
    let (files, now) = receiver();
    let refused = |mutate: &dyn Fn(&mut Vec<Vnode>, &mut Vec<FileIdentity>), why: &str| {
        let (mut files, mut now) = (files.clone(), now.clone());
        mutate(&mut files, &mut now);
        assert!(
            receiver_files(&files, SESSION, &fs(&now)).is_none(),
            "{why}"
        );
    };
    refused(&|f, _| f[1].nlink = 0, "unlinked database");
    refused(&|f, _| f[3].nlink = 0, "unlinked lock");
    refused(&|f, _| f[1].nlink = 2, "hardlinked database");
    refused(&|_, n| n[0].inode = 99, "database path replaced");
    refused(&|_, n| n[1].inode = 99, "lock path replaced");
    refused(&|_, n| drop(n.remove(0)), "database path vanished");
    refused(
        &|f, _| f[1].path = PathBuf::from("queue_1.sqlite"),
        "macOS 26 bare basename after a link removal",
    );
    refused(
        &|f, _| f[1].path = PathBuf::from("/queue_1.sqlite"),
        "macOS 27 bogus absolute name of a surviving link",
    );
    refused(
        &|f, _| f.push(vnode("/private/tmp/h/sqlite/queue_2.sqlite", 16777233, 20)),
        "another queue database",
    );
    refused(
        &|f, _| f.push(vnode("/queue_9.sqlite", 16777233, 21)),
        "a renamed surviving queue link",
    );
    refused(&|f, _| f[1].regular = false, "non-regular database");
    refused(&|f, _| drop(f.remove(3)), "no session lock");
    refused(&|f, _| drop(f.remove(1)), "no database");
}

#[test]
fn receiver_files_keep_distinct_connections_of_one_file_and_ignore_foreign_lock_dirs() {
    let (mut files, now) = receiver();
    files.push(vnode(DB, 16777233, 10));
    files.push(vnode(
        "/private/tmp/h/other/00000000-0000-0000-0000-000000000001.lock",
        16777233,
        30,
    ));
    let mut now = now;
    now.push(id(
        "/private/tmp/h/other/00000000-0000-0000-0000-000000000001.lock",
        16777233,
        30,
    ));
    assert!(receiver_files(&files, SESSION, &fs(&now)).is_some());
}

fn region(path: &str, inode: u64, executable: bool) -> Region {
    Region {
        vnode: vnode(path, 16777233, inode),
        executable,
    }
}
const EXE: &str = "/Users/u/.codex/packages/standalone/releases/x/codex";

#[test]
fn executable_requires_every_mapping_of_the_path_to_be_the_live_image() {
    let now = Some(id(EXE, 16777233, 7));
    let regions = vec![
        region(EXE, 7, true),
        region(EXE, 7, false),
        region("/usr/lib/dyld", 8, true),
        region("/private/tmp/copy-of-codex", 9, false),
    ];
    let p = Path::new(EXE);
    assert_eq!(
        executable(p, now.clone(), &regions, true, Some(p), now.clone()),
        now
    );
    // Replace after exec: the stale text region keeps the old inode under the same name.
    let mut replaced = regions.clone();
    replaced[0] = region(EXE, 6, true);
    replaced[0].vnode.nlink = 0;
    assert!(executable(p, now.clone(), &replaced, true, Some(p), now.clone()).is_none());
    // A stale region with nlink 0 but the same inode (unlinked image).
    let mut unlinked = regions.clone();
    unlinked[0].vnode.nlink = 0;
    assert!(executable(p, now.clone(), &unlinked, true, Some(p), now.clone()).is_none());
    // Only non-executable mappings of the path (a read-only mmap) never select.
    let read_only = vec![region(EXE, 7, false), region("/usr/lib/dyld", 8, true)];
    assert!(executable(p, now.clone(), &read_only, true, Some(p), now.clone()).is_none());
    // Other images alone never select, whatever their protection.
    assert!(executable(p, now.clone(), &regions[2..], true, Some(p), now.clone()).is_none());
    assert!(executable(p, now.clone(), &regions, false, Some(p), now.clone()).is_none());
    let renamed = Path::new("/elsewhere/codex");
    assert!(executable(p, now.clone(), &regions, true, Some(renamed), now.clone()).is_none());
    assert!(
        executable(
            p,
            now.clone(),
            &regions,
            true,
            Some(p),
            Some(id(EXE, 16777233, 8))
        )
        .is_none(),
        "path identity changed during the walk"
    );
    assert!(executable(p, None, &regions, true, Some(p), now.clone()).is_none());
    assert!(executable(p, now.clone(), &regions, true, None, now).is_none());
}

#[test]
fn helper_databases_report_every_live_connection_and_refuse_ambiguity() {
    let now = vec![id(DB, 1, 10)];
    let files = vec![
        vnode(DB, 1, 10),
        vnode(DB, 1, 10),
        vnode("/private/tmp/h/sqlite/state_5.sqlite", 1, 40),
    ];
    assert_eq!(
        helper_databases(&files, &fs(&now)).unwrap(),
        [(1, 10)].into_iter().collect()
    );
    let mut other = files.clone();
    other.push(vnode("/private/tmp/h/sqlite/queue_2.sqlite", 1, 41));
    assert_eq!(
        helper_databases(&other, &fs(&now)),
        Err("queue helper database ambiguous")
    );
    let mut relative = files.clone();
    relative[0].path = PathBuf::from("queue_1.sqlite");
    assert_eq!(
        helper_databases(&relative, &fs(&now)),
        Err("queue helper database ambiguous")
    );
    let mut unlinked = files.clone();
    unlinked[1].nlink = 0;
    assert_eq!(
        helper_databases(&unlinked, &fs(&now)),
        Err("queue helper database changed")
    );
    assert!(helper_databases(&files[2..], &fs(&now)).unwrap().is_empty());
}
