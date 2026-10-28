use super::*;
use crate::test_support::scratch::temp_path;

const KEY: &[u8] = b"SECRET-NODE-KEY-0123456789abcdef";

/// `<scratch>/plugins/herdr-threads` with a WAL node store at `schema`, `node.key` and
/// `node.json`. Returns (scratch, state).
fn state(tag: &str, schema: i64) -> (PathBuf, PathBuf) {
    let root = temp_path(format!("ht-backup-{tag}-{}", uuid::Uuid::new_v4()));
    let state = root.join("plugins").join("herdr-threads");
    let node = state.join("node");
    fs::create_dir_all(&node).unwrap();
    let db = Connection::open(node.join(DATABASE_FILE)).unwrap();
    db.execute_batch(&format!(
        "PRAGMA journal_mode=WAL; CREATE TABLE t(x); INSERT INTO t VALUES (1), (2); \
         PRAGMA user_version={schema};"
    ))
    .unwrap();
    drop(db);
    fs::write(node.join("node.key"), KEY).unwrap();
    fs::write(node.join("node.json"), b"{\"node\":1}").unwrap();
    (root, state)
}

fn backups(state: &Path) -> PathBuf {
    state.parent().unwrap().join(ROOT)
}

fn at(secs: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
}

/// 2026-10-28T12:00:00Z.
const NOW: u64 = 1_793_188_800;

/// A directory `name` under `root` holding a store at `schema` (none when `None`).
fn entry(root: &Path, name: &str, schema: Option<i64>) -> PathBuf {
    let dir = root.join(name);
    fs::create_dir_all(&dir).unwrap();
    if let Some(schema) = schema {
        let db = Connection::open(dir.join(DATABASE_FILE)).unwrap();
        db.execute_batch(&format!("PRAGMA user_version={schema};"))
            .unwrap();
    }
    dir
}

fn names(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn taken(outcome: Outcome) -> (PathBuf, i64, PruneReport) {
    match outcome {
        Outcome::Taken {
            backup,
            schema,
            prune,
        } => (backup, schema, prune),
        Outcome::NoStore => panic!("no backup taken"),
    }
}

#[test]
fn takes_a_checked_private_copy_of_store_key_and_node_json() {
    let (_root, state) = state("take", 9);
    // A live connection with a committed WAL row, as a running daemon has.
    let live = Connection::open(state.join("node").join(DATABASE_FILE)).unwrap();
    live.execute_batch("PRAGMA wal_autocheckpoint=0; INSERT INTO t VALUES (3);")
        .unwrap();
    let (backup, schema, prune) = taken(backup_and_prune(&state, "v2.5.0", at(NOW)).unwrap());
    drop(live);
    assert_eq!(schema, 9);
    assert!(prune.pruned.is_empty() && prune.unreadable.is_empty());
    assert_eq!(backup, backups(&state).join("pre-v2.5.0-20261028T120000Z"));
    let copy = Connection::open(backup.join(DATABASE_FILE)).unwrap();
    let rows: i64 = copy
        .query_row("SELECT count(*) FROM t", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 3, "the copy holds the WAL's committed rows");
    drop(copy);
    assert_eq!(fs::read(backup.join("node.key")).unwrap(), KEY);
    assert_eq!(fs::read(backup.join("node.json")).unwrap(), b"{\"node\":1}");
    #[cfg(unix)]
    {
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&backups(&state)), 0o700);
        assert_eq!(mode(&backup), 0o700);
        for name in [DATABASE_FILE, "node.key", "node.json"] {
            assert_eq!(mode(&backup.join(name)), 0o600, "{name}");
        }
    }
}

#[test]
fn a_taken_name_gets_a_numbered_suffix() {
    let (_root, state) = state("suffix", 9);
    let first = taken(backup_and_prune(&state, "2.5.0", at(NOW)).unwrap()).0;
    let second = taken(backup_and_prune(&state, "2.5.0", at(NOW)).unwrap()).0;
    assert!(first.ends_with("pre-v2.5.0-20261028T120000Z"));
    assert!(second.ends_with("pre-v2.5.0-20261028T120000Z-2"));
}

#[test]
fn no_store_takes_no_backup() {
    let root = temp_path(format!("ht-backup-none-{}", uuid::Uuid::new_v4()));
    let state = root.join("herdr-threads");
    fs::create_dir_all(&state).unwrap();
    assert!(matches!(
        backup_and_prune(&state, "v2.5.0", at(NOW)).unwrap(),
        Outcome::NoStore
    ));
    assert!(!backups(&state).exists());
}

#[test]
fn a_store_that_is_not_sqlite_fails_and_leaves_no_staging() {
    let (_root, state) = state("corrupt", 9);
    let node = state.join("node");
    let _ = fs::remove_file(node.join("herdr-threads.db-wal"));
    let _ = fs::remove_file(node.join("herdr-threads.db-shm"));
    fs::write(node.join(DATABASE_FILE), b"not a database at all").unwrap();
    let error = backup_and_prune(&state, "v2.5.0", at(NOW)).unwrap_err();
    assert!(!error.contains("SECRET"), "{error}");
    assert!(names(&backups(&state)).is_empty(), "staging removed");
}

#[test]
fn a_backup_root_that_is_not_a_directory_is_refused() {
    let (_root, state) = state("rootfile", 9);
    fs::write(backups(&state), b"").unwrap();
    assert!(backup_and_prune(&state, "v2.5.0", at(NOW)).is_err());
}

#[test]
fn an_invalid_version_is_refused() {
    let (_root, state) = state("tag", 9);
    for version in ["", "v", "v2.5.0/../x", "v2 5"] {
        assert!(
            backup_and_prune(&state, version, at(NOW)).is_err(),
            "{version:?}"
        );
    }
}

#[test]
fn prune_keeps_newest_three_newest_per_schema_and_unreadable() {
    let (_root, state) = state("prune", 9);
    let root = backups(&state);
    fs::create_dir_all(&root).unwrap();
    // Oldest to newest by name stamp; the new backup (schema 9) is newest.
    entry(&root, "pre-v2.0.1-20261001T000000Z", Some(1)); // newest schema 1: kept
    entry(&root, "pre-v2.1.0-20261002T000000Z", Some(2)); // pruned
    entry(&root, "pre-v2.1.1-20261003T000000Z", Some(2)); // newest schema 2: kept
    entry(&root, "pre-v2.2.0-20261004T000000Z", None); // unreadable: kept
    entry(&root, "pre-v2.4.1-20261005T000000Z", Some(5)); // pruned
    entry(&root, "pre-v2.4.1-20261005T000000Z-2", Some(5)); // pruned
    entry(&root, "pre-v2.4.2-20261006T000000Z", Some(5)); // third newest: kept
    entry(&root, "pre-v2.4.3-20261007T000000Z", Some(9)); // second newest: kept
    // Off the grammar: never touched, whatever they hold.
    entry(&root, "pre-v2.0.0_20261001T000000Z", Some(1));
    entry(&root, "pre-v2.0.0-20261001T0000Z", Some(1));
    entry(&root, "notes", None);
    let plugins = state.parent().unwrap();
    let manual = entry(
        plugins,
        "herdr-threads-pre-v2.0.1-backup-20261017T160720Z",
        Some(1),
    );
    // A symlink on the grammar is never followed or removed.
    let target = entry(&root, "keep-target", Some(3));
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, root.join("pre-v1.0.0-20250101T000000Z")).unwrap();

    let (backup, _, prune) = taken(backup_and_prune(&state, "v2.5.0", at(NOW)).unwrap());
    let mut pruned: Vec<String> = prune
        .pruned
        .iter()
        .map(|path| path.file_name().unwrap().to_str().unwrap().to_owned())
        .collect();
    pruned.sort();
    assert_eq!(
        pruned,
        vec![
            "pre-v2.1.0-20261002T000000Z",
            "pre-v2.4.1-20261005T000000Z",
            "pre-v2.4.1-20261005T000000Z-2",
        ]
    );
    assert_eq!(prune.unreadable.len(), 1);
    assert!(
        prune.unreadable[0]
            .0
            .ends_with("pre-v2.2.0-20261004T000000Z")
    );
    assert!(prune.failed.is_empty());
    let mut expected = vec![
        backup.file_name().unwrap().to_str().unwrap(),
        "keep-target",
        "notes",
        "pre-v2.0.0-20261001T0000Z",
        "pre-v2.0.0_20261001T000000Z",
        "pre-v2.0.1-20261001T000000Z",
        "pre-v2.1.1-20261003T000000Z",
        "pre-v2.2.0-20261004T000000Z",
        "pre-v2.4.2-20261006T000000Z",
        "pre-v2.4.3-20261007T000000Z",
    ];
    #[cfg(unix)]
    expected.push("pre-v1.0.0-20250101T000000Z");
    expected.sort();
    assert_eq!(names(&root), expected);
    assert!(
        manual.join(DATABASE_FILE).is_file(),
        "manual backups beside the state are untouched"
    );
    assert!(target.join(DATABASE_FILE).is_file());
}

#[test]
fn the_backup_just_taken_survives_when_newer_names_exist() {
    let (_root, state) = state("taken", 9);
    let root = backups(&state);
    fs::create_dir_all(&root).unwrap();
    // A clock that went backwards: three newer entries with the new backup's schema.
    for day in 1..=3 {
        entry(
            &root,
            &format!("pre-v3.0.{day}-2030010{day}T000000Z"),
            Some(9),
        );
    }
    let (backup, _, prune) = taken(backup_and_prune(&state, "v2.5.0", at(NOW)).unwrap());
    assert!(backup.join(DATABASE_FILE).is_file());
    assert!(prune.pruned.is_empty());
    assert_eq!(names(&root).len(), 4);
}

#[test]
fn reading_a_schema_creates_no_sidecars_and_changes_no_mtime() {
    let root = temp_path(format!("ht-backup-ro-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store = root.join(DATABASE_FILE);
    let db = Connection::open(&store).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE t(x); PRAGMA user_version=7;")
        .unwrap();
    drop(db);
    let before = fs::metadata(&store).unwrap().modified().unwrap();
    let listing = names(&root);
    assert_eq!(user_version(&store), Ok(7));
    assert_eq!(names(&root), listing);
    assert_eq!(fs::metadata(&store).unwrap().modified().unwrap(), before);
}

#[test]
fn entry_key_accepts_only_the_grammar() {
    assert_eq!(
        entry_key("pre-v2.5.0-20261028T120000Z"),
        Some((20261028120000, 1))
    );
    assert_eq!(
        entry_key("pre-v2.5.0-rc.1-20261028T120000Z-3"),
        Some((20261028120000, 3))
    );
    for name in [
        "pre-v-20261028T120000Z",
        "pre-v2.5.0-20261028T120000",
        "pre-v2.5.0-20261028-120000Z",
        "pre-v2.5.0_20261028T120000Z",
        "pre-v2.5.0-20261028T120000Z-",
        "pre-v2.5.0-20261028T120000Z-x",
        "pre-v2.5.0-2026102AT120000Z",
        "pre-vé-20261028T120000Z",
        "herdr-threads-pre-v2.5.0-backup-20261028T120000Z",
        ".staging-42",
    ] {
        assert_eq!(entry_key(name), None, "{name}");
    }
}

#[test]
fn a_staging_directory_left_by_a_dead_run_is_swept() {
    let (_root, state) = state("sweep", 9);
    let root = backups(&state);
    fs::create_dir_all(&root).unwrap();
    // No process has pid i32::MAX; this process is alive and is not the stale owner.
    let dead = entry(&root, &format!("{STAGING_PREFIX}{}", i32::MAX), Some(9));
    let live_pid = std::os::unix::process::parent_id();
    let live = entry(&root, &format!("{STAGING_PREFIX}{live_pid}"), Some(9));
    taken(backup_and_prune(&state, "v2.5.0", at(NOW)).unwrap());
    assert!(!dead.exists());
    assert!(live.exists(), "a live run's staging is left alone");
}

#[test]
fn a_copy_that_fails_its_integrity_check_is_refused() {
    let root = temp_path(format!("ht-backup-check-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store = root.join(DATABASE_FILE);
    let db = Connection::open(&store).unwrap();
    db.execute_batch(
        "CREATE TABLE t(x INTEGER); CREATE INDEX t_x ON t(x); \
         INSERT INTO t VALUES (1), (2), (3);",
    )
    .unwrap();
    drop(db);
    assert_eq!(check_copy(&store), Ok(0));
    // Point the index's definition at another expression: the file still opens and reads,
    // but the stored index entries no longer match the table.
    let db = Connection::open(&store).unwrap();
    db.execute_batch(
        "PRAGMA writable_schema=ON; \
         UPDATE sqlite_schema SET sql = 'CREATE INDEX t_x ON t(x + 1)' WHERE name = 't_x'; \
         PRAGMA writable_schema=OFF;",
    )
    .unwrap();
    drop(db);
    let error = check_copy(&store).unwrap_err();
    assert!(
        error.starts_with("the copy failed its integrity check"),
        "{error}"
    );
}
