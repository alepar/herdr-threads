//! `SeatName` against the 0012 migration's `seats.name` CHECK. The CHECK is a
//! superset of the grammar (length and charset); code adds the leading `-`/`.`
//! and seat-id-shaped rules. So: whatever `SeatName` accepts the CHECK accepts,
//! and whatever the CHECK rejects `SeatName` rejects.

use herdr_threads::protocol::ids::SeatName;
use rusqlite::Connection;

/// An in-memory `seats` stub that gains its `name` column from the migration's
/// own statement, so the CHECK under test is the shipped one.
fn seats_with_name_column() -> Connection {
    let migration = include_str!("../migrations/node/0012_host_environments.sql");
    let start = migration
        .find("ALTER TABLE seats ADD COLUMN name")
        .expect("0012 adds seats.name");
    let statement = &migration[start..start + migration[start..].find(';').unwrap() + 1];
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE seats (ordinal INTEGER PRIMARY KEY);")
        .unwrap();
    db.execute_batch(statement).unwrap();
    db
}

fn check_accepts(db: &Connection, name: &str) -> bool {
    let inserted = db.execute("INSERT INTO seats(name) VALUES (?1)", [name]);
    inserted.is_ok()
}

fn corpus() -> Vec<String> {
    let mut names: Vec<String> = [
        "reviewer",
        "a",
        "a.b_c-1",
        "A-Z_0.9",
        "x-",
        "x.",
        "",
        "-x",
        ".x",
        "..",
        "a b",
        "a/b",
        "a:b",
        "a\u{0}b",
        "\u{e9}",
        "caf\u{e9}",
        "a\n",
        " a",
        // seat-id shaped: `s`, a 3-character node tag, 8 base62 characters
        "sQ7ak3Fq9a2B",
        "sAAA00000001",
        // not seat-id shaped: one character too long or short
        "sQ7ak3Fq9a2",
        "sQ7ak3Fq9a2BC",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    names.push("a".repeat(64));
    names.push("a".repeat(65));
    names.push("\u{e9}".repeat(32)); // 64 bytes, not ASCII
    names
}

#[test]
fn every_accepted_name_passes_the_check() {
    let db = seats_with_name_column();
    let mut accepted = 0;
    for name in corpus() {
        if SeatName::parse(name.clone()).is_ok() {
            accepted += 1;
            assert!(check_accepts(&db, &name), "CHECK rejects accepted {name:?}");
        }
    }
    assert!(accepted >= 6, "the corpus must exercise accepted names");
}

#[test]
fn every_name_the_check_rejects_is_rejected() {
    let db = seats_with_name_column();
    let mut rejected = 0;
    for name in corpus() {
        if !check_accepts(&db, &name) {
            rejected += 1;
            assert!(
                SeatName::parse(name.clone()).is_err(),
                "SeatName accepts {name:?}, which the CHECK rejects"
            );
        }
    }
    assert!(rejected >= 6, "the corpus must exercise rejected names");
}

#[test]
fn grammar_only_rules_are_in_code() {
    let db = seats_with_name_column();
    for name in ["-x", ".x", "sQ7ak3Fq9a2B"] {
        assert!(check_accepts(&db, name), "the CHECK is a superset: {name}");
        assert!(SeatName::parse(name).is_err(), "{name}");
    }
    assert_eq!(SeatName::parse("a".repeat(64)).unwrap().as_str().len(), 64);
    assert!(SeatName::parse("a".repeat(65)).is_err());
}
