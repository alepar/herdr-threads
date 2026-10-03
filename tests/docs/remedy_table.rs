//! ht-p03.36: the remedy table in `docs/operations.md` is the text `remedy()`
//! produces, not a hand-maintained copy. Mounted from `src/daemon/remedy.rs`.
//!
//! The table is compared, not generated: the test parses the rows between the
//! `remedy-table` markers and fails with the expected row when one differs, so
//! a changed remedy string fails here until the document is edited to match.
use super::*;
use std::collections::BTreeSet;
use std::path::PathBuf;

const OPERATIONS_MD: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/operations.md");
/// The log path placeholders the table renders: the startup contexts name the
/// attempt's own log, the lane and doctor contexts name `daemon.log`.
const STARTUP_LOG: &str = "<instance>/logs/startup-<pid>-<nonce>.log";
const DAEMON_LOG: &str = "<instance>/daemon.log";

fn class_named(name: &str) -> ErrorClass {
    match name {
        "Transient" => ErrorClass::Transient,
        "Unavailable" => ErrorClass::Unavailable,
        "Corrupt" => ErrorClass::Corrupt,
        "VersionSkew" => ErrorClass::VersionSkew,
        other => panic!("remedy table names an unknown class {other:?}"),
    }
}

fn context_named(name: &str) -> RemedyContext {
    match name {
        "StartupFailure" => RemedyContext::StartupFailure {
            log: PathBuf::from(STARTUP_LOG),
        },
        "LaneDegraded" => RemedyContext::LaneDegraded {
            log: PathBuf::from(DAEMON_LOG),
        },
        "Exit3" => RemedyContext::Exit3,
        "VersionSkew" => RemedyContext::VersionSkew {
            daemon: "<daemon>".to_owned(),
            cli: "<cli>".to_owned(),
        },
        other => panic!("remedy table names an unknown context {other:?}"),
    }
}

/// `(class, context, remedy)` per data row between the markers.
fn table_rows() -> Vec<(String, String, String)> {
    let text = std::fs::read_to_string(OPERATIONS_MD).expect("docs/operations.md");
    let begin = "<!-- remedy-table:begin -->";
    let end = "<!-- remedy-table:end -->";
    let block = text
        .split_once(begin)
        .and_then(|(_, rest)| rest.split_once(end))
        .expect("operations.md has the remedy-table markers")
        .0;
    block
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('|'))
        // Header and separator rows.
        .skip(2)
        .map(|line| {
            let cells: Vec<&str> = line
                .trim_matches('|')
                .splitn(3, '|')
                .map(str::trim)
                .collect();
            assert_eq!(cells.len(), 3, "row needs class | context | remedy: {line}");
            (
                cells[0].trim_matches('`').to_owned(),
                cells[1].trim_matches('`').to_owned(),
                cells[2].to_owned(),
            )
        })
        .collect()
}

/// Kills: a remedy string edited in `remedy.rs` without the document, and a
/// document row edited by hand away from the code's text.
#[test]
fn every_remedy_row_equals_remedy_output() {
    let rows = table_rows();
    assert!(!rows.is_empty(), "the remedy table has no rows");
    for (class, context, text) in rows {
        let expected = remedy(Some(class_named(&class)), &context_named(&context));
        assert_eq!(text, expected, "row {class} / {context}");
    }
}

/// Kills: an `ErrorClass` or a context losing its row, so an operator reads
/// no remedy for a failure the daemon can report. One row per class at every
/// context that varies by class, plus the version-skew sentence.
#[test]
fn table_has_a_row_per_class_and_context() {
    let have: BTreeSet<(String, String)> = table_rows()
        .into_iter()
        .map(|(class, context, _)| (class, context))
        .collect();
    let mut want = BTreeSet::new();
    for class in ["Transient", "Unavailable", "Corrupt", "VersionSkew"] {
        for context in ["StartupFailure", "LaneDegraded", "Exit3"] {
            want.insert((class.to_owned(), context.to_owned()));
        }
    }
    want.insert(("VersionSkew".to_owned(), "VersionSkew".to_owned()));
    assert_eq!(have, want);
}

/// The table folds two contexts into the rows above; this keeps that honest.
/// Kills: a timeout or doctor remedy diverging from its row's text.
#[test]
fn folded_contexts_share_their_rows_text() {
    let startup = PathBuf::from(STARTUP_LOG);
    let daemon = PathBuf::from(DAEMON_LOG);
    for class in [
        ErrorClass::Transient,
        ErrorClass::Unavailable,
        ErrorClass::Corrupt,
        ErrorClass::VersionSkew,
    ] {
        assert_eq!(
            remedy(
                Some(class),
                &RemedyContext::StartupTimeout {
                    log: startup.clone()
                }
            ),
            remedy(
                Some(class),
                &RemedyContext::StartupFailure {
                    log: startup.clone()
                }
            ),
            "{class:?}"
        );
        assert_eq!(
            remedy(
                Some(class),
                &RemedyContext::Doctor {
                    log: daemon.clone()
                }
            ),
            remedy(
                Some(class),
                &RemedyContext::LaneDegraded {
                    log: daemon.clone()
                }
            ),
            "{class:?}"
        );
    }
}
