//! ht-p03.10: the four-way error taxonomy (SQLite and connect classification)
//! and the single `remedy()` text table. Mounted from `src/daemon/remedy.rs`.
use super::*;
use crate::client::classify_connect_error;
use crate::protocol::results::ErrorCode;
use crate::store::connection::store_error;
use rusqlite::ffi;
use std::io;

fn sqlite(code: i32) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(ffi::Error::new(code), Some("injected".into()))
}

/// Kills: any unlisted code mapping to StoreCorrupt (the old `_ =>` arm), a
/// listed code losing its row, and the unlisted detail losing its marker.
#[test]
fn sqlite_error_mapping_table() {
    const LISTED: &[(i32, ErrorCode, Option<ErrorClass>)] = &[
        (
            ffi::SQLITE_BUSY,
            ErrorCode::StoreBusy,
            Some(ErrorClass::Transient),
        ),
        (
            ffi::SQLITE_LOCKED,
            ErrorCode::StoreBusy,
            Some(ErrorClass::Transient),
        ),
        (
            ffi::SQLITE_CORRUPT,
            ErrorCode::StoreCorrupt,
            Some(ErrorClass::Corrupt),
        ),
        (
            ffi::SQLITE_NOTADB,
            ErrorCode::StoreCorrupt,
            Some(ErrorClass::Corrupt),
        ),
        (
            ffi::SQLITE_FULL,
            ErrorCode::StoreFull,
            Some(ErrorClass::Transient),
        ),
        (ffi::SQLITE_CONSTRAINT, ErrorCode::Conflict, None),
        (ffi::SQLITE_INTERRUPT, ErrorCode::Cancelled, None),
    ];
    for (raw, code, class) in LISTED {
        let error = store_error(sqlite(*raw));
        assert_eq!(error.code, *code, "sqlite code {raw}");
        assert_eq!(error.class(), *class, "sqlite code {raw}");
    }
    // Unlisted codes: Transient with detail, never Corrupt.
    for raw in [
        ffi::SQLITE_IOERR,
        ffi::SQLITE_READONLY,
        ffi::SQLITE_CANTOPEN,
        ffi::SQLITE_NOMEM,
        ffi::SQLITE_PERM,
        ffi::SQLITE_PROTOCOL,
        ffi::SQLITE_SCHEMA,
        ffi::SQLITE_TOOBIG,
        ffi::SQLITE_MISMATCH,
        ffi::SQLITE_MISUSE,
        ffi::SQLITE_AUTH,
        ffi::SQLITE_RANGE,
        ffi::SQLITE_ERROR,
        ffi::SQLITE_INTERNAL,
        ffi::SQLITE_ABORT,
        ffi::SQLITE_NOTFOUND,
        ffi::SQLITE_FORMAT,
        ffi::SQLITE_NOLFS,
    ] {
        let error = store_error(sqlite(raw));
        assert_ne!(error.code, ErrorCode::StoreCorrupt, "sqlite code {raw}");
        assert_eq!(error.class(), Some(ErrorClass::Transient), "code {raw}");
        assert!(
            error
                .detail
                .starts_with("SQLite (unclassified, transient): "),
            "{}",
            error.detail
        );
        assert!(error.detail.contains("injected"), "{}", error.detail);
    }
    // A non-SqliteFailure error is the same unclassified transient.
    let other = store_error(rusqlite::Error::QueryReturnedNoRows);
    assert_eq!(other.code, ErrorCode::StoreBusy);
    assert_eq!(other.class(), Some(ErrorClass::Transient));
    assert!(
        other
            .detail
            .starts_with("SQLite (unclassified, transient): ")
    );
}

/// A failed integrity check is Corrupt (and an integrity check that cannot run
/// because the page is unreadable is SQLITE_CORRUPT, also Corrupt).
#[test]
fn failed_integrity_check_is_corrupt() {
    let dir = std::env::temp_dir().join(format!("ht-integrity-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("db.sqlite3");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE t(a INTEGER PRIMARY KEY, b TEXT);
             CREATE INDEX t_b ON t(b);",
        )
        .unwrap();
        for i in 0..400 {
            conn.execute("INSERT INTO t(b) VALUES (?1)", [format!("row-{i:0>40}")])
                .unwrap();
        }
        assert!(
            crate::store::schema::check_integrity(&conn).is_ok(),
            "an undamaged database passes"
        );
    }
    let mut bytes = std::fs::read(&path).unwrap();
    assert!(bytes.len() > 4096 * 3);
    for byte in &mut bytes[4096 * 2 + 16..4096 * 2 + 2048] {
        *byte = 0xA5;
    }
    std::fs::write(&path, bytes).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    let error = crate::store::schema::check_integrity(&conn).unwrap_err();
    assert_eq!(error.code, ErrorCode::StoreCorrupt, "{}", error.detail);
    assert_eq!(error.class(), Some(ErrorClass::Corrupt));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn connect_error_classification_table() {
    use io::ErrorKind::*;
    let table = [
        (
            ConnectionRefused,
            ErrorClass::Unavailable,
            "server not running",
        ),
        (NotFound, ErrorClass::Unavailable, "server not running"),
        (TimedOut, ErrorClass::Transient, "connect timed out"),
        (WouldBlock, ErrorClass::Transient, "connect timed out"),
        (Interrupted, ErrorClass::Transient, "connect timed out"),
        (
            PermissionDenied,
            ErrorClass::Unavailable,
            "socket refused by sandbox",
        ),
        (BrokenPipe, ErrorClass::Unavailable, "connection failed"),
    ];
    for (kind, class, text) in table {
        assert_eq!(
            classify_connect_error(&io::Error::from(kind)),
            (class, text),
            "{kind:?}"
        );
    }
    // The client's own connect error carries the classification text.
    let refused = crate::client::connect_error(
        &io::Error::from(ConnectionRefused),
        std::path::Path::new("/x/d.sock"),
    );
    assert_eq!(refused.code, ErrorCode::HostUnavailable);
    assert!(refused.detail.contains("server not running"), "{refused:?}");
}

fn contexts(log: &std::path::Path) -> Vec<RemedyContext> {
    vec![
        RemedyContext::StartupFailure {
            log: log.to_path_buf(),
        },
        RemedyContext::StartupTimeout {
            log: log.to_path_buf(),
        },
        RemedyContext::Exit3,
        RemedyContext::LaneDegraded {
            log: log.to_path_buf(),
        },
        RemedyContext::VersionSkew {
            daemon: "0.0.9 (protocol 7)".into(),
            cli: "0.1.0 (protocol 1)".into(),
        },
        RemedyContext::Doctor {
            log: log.to_path_buf(),
        },
    ]
}

/// Every class (and no class) at every context: non-empty, log named where a
/// log is the next step, exit 3 names `ensure` for an unavailable daemon, `stop` then `ensure` for
/// version skew, skew names
/// both versions and both commands, Corrupt says to back up first.
#[test]
fn remedy_text_table() {
    let log = PathBuf::from("/state/instances/abc/daemon.log");
    let classes = [
        None,
        Some(ErrorClass::Transient),
        Some(ErrorClass::Unavailable),
        Some(ErrorClass::Corrupt),
        Some(ErrorClass::VersionSkew),
    ];
    for class in classes {
        for context in contexts(&log) {
            let text = remedy(class, &context);
            assert!(!text.is_empty(), "{class:?} {context:?}");
            match &context {
                RemedyContext::Exit3 => {
                    if class == Some(ErrorClass::VersionSkew) {
                        let stop = text.find("daemon stop").expect(&text);
                        let ensure = text.find("daemon ensure").expect(&text);
                        assert!(stop < ensure, "stop comes first: {text}");
                    } else {
                        assert!(text.contains("daemon ensure"), "{text}");
                        assert!(!text.contains("daemon stop"), "{text}");
                    }
                }
                RemedyContext::VersionSkew { daemon, cli } => {
                    assert_eq!(
                        text,
                        format!(
                            "daemon is version {daemon}, CLI is {cli}: run `herdr-threads daemon stop` then `herdr-threads daemon ensure`"
                        )
                    );
                }
                RemedyContext::StartupFailure { .. }
                | RemedyContext::StartupTimeout { .. }
                | RemedyContext::LaneDegraded { .. }
                | RemedyContext::Doctor { .. } => {
                    assert!(text.contains(&log.display().to_string()), "{text}");
                    if class == Some(ErrorClass::Corrupt) {
                        assert!(text.contains("back up /state/instances/abc"), "{text}");
                    }
                }
            }
        }
    }
    assert!(
        remedy(
            Some(ErrorClass::Unavailable),
            &RemedyContext::StartupFailure { log: log.clone() }
        )
        .contains("the daemon did not start")
    );
    assert!(
        remedy(
            Some(ErrorClass::Transient),
            &RemedyContext::Doctor { log: log.clone() }
        )
        .starts_with("temporary; retry the command")
    );
    // A startup log lives under logs/: the state to back up is its parent.
    let startup = PathBuf::from("/state/instances/abc/logs/startup-1-ab.log");
    assert!(
        remedy(
            Some(ErrorClass::Corrupt),
            &RemedyContext::StartupFailure { log: startup }
        )
        .contains("back up /state/instances/abc before repair")
    );
}

/// The former hand-written remedy literals are gone from their sites; every
/// remedy string comes from `remedy()`.
#[test]
fn former_hand_written_remedy_literals_are_gone() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let banned = [
        "matching older executable",
        "with the older executable",
        "inspect daemon health and logs",
        "inspect `daemon health` and logs",
        "run `daemon stop`",
        "run `herdr-threads daemon ensure` and retry",
        "same state/host context",
    ];
    for file in [
        "src/daemon/lifecycle.rs",
        "src/daemon/control.rs",
        "src/cli/doctor.rs",
        "src/cli/exit.rs",
        "src/cli/commands.rs",
    ] {
        let text = std::fs::read_to_string(root.join(file)).unwrap();
        for literal in banned {
            assert!(
                !text.contains(literal),
                "{file} still hand-writes the remedy {literal:?}; use daemon::remedy::remedy"
            );
        }
    }
    // Sanity: the scan would notice the new text living in the one module.
    let remedy_source = std::fs::read_to_string(root.join("src/daemon/remedy.rs")).unwrap();
    assert!(remedy_source.contains("daemon stop"));
}

#[test]
fn exit_3_remedy_is_chosen_by_class() {
    assert_eq!(
        remedy(Some(ErrorClass::Unavailable), &RemedyContext::Exit3),
        "run `herdr-threads daemon ensure`"
    );
    assert_eq!(
        remedy(None, &RemedyContext::Exit3),
        remedy(Some(ErrorClass::Unavailable), &RemedyContext::Exit3)
    );
    assert_eq!(
        remedy(Some(ErrorClass::VersionSkew), &RemedyContext::Exit3),
        "run `herdr-threads daemon stop` then `herdr-threads daemon ensure`"
    );
    // The skew context names the versions, then the same commands.
    assert!(
        remedy(
            None,
            &RemedyContext::VersionSkew {
                daemon: "a".into(),
                cli: "b".into()
            }
        )
        .ends_with(&remedy(
            Some(ErrorClass::VersionSkew),
            &RemedyContext::Exit3
        ))
    );
}
