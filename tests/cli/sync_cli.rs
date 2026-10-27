//! ht-5fr.8: `peer add|forget|list` and `sync status|verify|unapplied|resync|
//! clone-cleared` through the installed executable, against a real store-only
//! daemon each test starts and stops. Each test names what it kills.

use herdr_threads::test_support::isolation::TestIsolation;
use rusqlite::Connection;
use std::{
    path::{Path, PathBuf},
    process::Output,
    time::Duration,
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

struct Fixture {
    iso: TestIsolation,
    state: PathBuf,
}

impl Fixture {
    /// A fresh state directory with its daemon running.
    fn new(tag: &str) -> Self {
        Self::start(tag, None)
    }

    /// As [`Fixture::new`], with `node/settings.json` written first.
    fn with_settings(tag: &str, json: &str) -> Self {
        Self::start(tag, Some(json))
    }

    fn start(tag: &str, settings: Option<&str>) -> Self {
        let iso = TestIsolation::new(tag);
        let state = iso.path("state");
        if let Some(json) = settings {
            use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(state.join("node"))
                .unwrap();
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(state.join("node").join("settings.json"))
                .unwrap();
            std::io::Write::write_all(&mut file, json.as_bytes()).unwrap();
        }
        let fixture = Self { iso, state };
        let ensured = fixture
            .command(false)
            .args(["daemon", "ensure"])
            .output()
            .unwrap();
        assert!(ensured.status.success(), "{ensured:?}");
        fixture
    }

    fn command(&self, human: bool) -> std::process::Command {
        let mut command = self.iso.command(BIN);
        if human {
            command.arg("human");
        }
        // `PATH` holds no `herdr`: the host resolves to none, deterministically.
        command
            .env("PATH", "/usr/bin:/bin")
            .arg("--state-dir")
            .arg(&self.state);
        command
    }

    /// The agent form (a read, or a refused operator command).
    fn run(&self, args: &[&str]) -> Output {
        self.command(false).args(args).output().unwrap()
    }

    /// The operator form: `human ...`.
    fn human(&self, args: &[&str]) -> Output {
        self.command(true).args(args).output().unwrap()
    }

    /// A writer on the live daemon's store, for seeding state no CLI can make.
    fn db(&self) -> Connection {
        let db = Connection::open(self.state.join("node").join("herdr-threads.db")).unwrap();
        db.busy_timeout(Duration::from_secs(10)).unwrap();
        db
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.command(false).args(["daemon", "stop"]).output();
    }
}

fn out(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn err(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn ok(output: &Output) -> String {
    assert!(output.status.success(), "{output:?}");
    out(output)
}

fn refused(output: &Output) -> String {
    assert!(!output.status.success(), "{output:?}");
    err(output)
}

// Kills: a node-scoped command that needs a state-dir flag or a Herdr host
// (exit 2 "host endpoint missing"): the context comes from
// HERDR_PLUGIN_STATE_DIR alone.
#[test]
fn sync_and_peer_commands_run_with_env_state_dir_and_no_host() {
    let fx = Fixture::new("sync-cli-no-host");
    let env_cmd = |args: &[&str]| {
        fx.iso
            .command(BIN)
            .env("PATH", "/usr/bin:/bin")
            .env("HERDR_PLUGIN_STATE_DIR", &fx.state)
            .args(args)
            .output()
            .unwrap()
    };
    assert!(ok(&env_cmd(&["sync", "status"])).starts_with("sync_status"));
    assert_eq!(
        ok(&env_cmd(&["peer", "list"])).lines().next(),
        Some("peers")
    );
    ok(&env_cmd(&["daemon", "health"]));
    ok(&env_cmd(&["daemon", "stop"]));
}

// Kills: an add that is not listed, a list that hides a forgotten target, a
// forget that deletes the row, and a forget of nothing that succeeds.
#[test]
fn peer_add_list_forget_round_trip() {
    let fx = Fixture::new("sync-cli-peers");
    let first = ok(&fx.run(&["peer", "list"]));
    let first_lines: Vec<_> = first.lines().collect();
    assert_eq!(first_lines.len(), 3, "{first}");
    assert_eq!(first_lines[0], "peers", "{first}");
    assert!(
        [
            "discovery starting",
            "discovery no_host",
            "discovery running"
        ]
        .contains(&first_lines[1]),
        "{first}"
    );
    assert_eq!(first_lines[2], "empty", "{first}");

    let added = ok(&fx.human(&[
        "peer",
        "add",
        "alice@host",
        "--remote-command",
        "ht --state-dir /x",
    ]));
    assert!(added.starts_with("peer_added "), "{added}");
    assert!(added.trim_end().ends_with(": alice@host"), "{added}");

    let listed = ok(&fx.run(&["peer", "list"]));
    let rows: Vec<_> = listed.lines().filter(|l| l.starts_with("peer ")).collect();
    assert_eq!(rows.len(), 1, "{listed}");
    assert!(
        rows[0].ends_with(": alice@host cmd=ht --state-dir /x"),
        "{listed}"
    );
    assert!(rows[0].contains(" explicit "), "{listed}");
    assert!(rows[0].contains("node=- reach=unknown"), "{listed}");

    assert_eq!(
        ok(&fx.human(&["peer", "forget", "alice@host"])),
        "peer_forgotten 1\n"
    );
    let listed = ok(&fx.run(&["peer", "list"]));
    let row = listed.lines().find(|l| l.starts_with("peer ")).unwrap();
    assert!(row.contains(" explicit off "), "kept, disabled: {listed}");
    let again = refused(&fx.human(&["peer", "forget", "alice@host"]));
    assert!(again.contains("no enabled dial target matches"), "{again}");
}

// Kills: control characters or an ssh option reaching the table, and a
// refused add that leaves a row behind.
#[test]
fn peer_add_refuses_control_chars_and_options() {
    let fx = Fixture::new("sync-cli-refuse");
    let control = refused(&fx.human(&["peer", "add", "a\nb@host"]));
    assert!(control.contains("control characters"), "{control}");
    let escape = refused(&fx.human(&["peer", "add", "a\u{1b}[31m@host"]));
    assert!(escape.contains("control characters"), "{escape}");
    let command = refused(&fx.human(&["peer", "add", "ok@host", "--remote-command", "x\ny"]));
    assert!(command.contains("control characters"), "{command}");
    // `--` ends option parsing, so the dash reaches our own check.
    let option = refused(&fx.human(&["peer", "add", "--", "-oProxyCommand=x"]));
    assert!(option.contains("must not start with '-'"), "{option}");
    let listed = ok(&fx.run(&["peer", "list"]));
    assert!(listed.starts_with("peers\n"), "{listed}");
    assert!(listed.ends_with("empty\n"), "{listed}");
    assert!(
        listed.lines().all(|l| !l.starts_with("peer ")),
        "no rows: {listed}"
    );
}

// Kills: an operator command that an agent-form call can run, a refusal that
// does not name the `human` form, and a refused call that changed state.
#[test]
fn mutating_commands_operator_gated() {
    let fx = Fixture::new("sync-cli-gate");
    let mutating: [&[&str]; 5] = [
        &["peer", "add", "a@h"],
        &["peer", "forget", "a@h"],
        &["sync", "unapplied", "replay"],
        &["sync", "resync", "--from", "Pq7"],
        &["sync", "clone-cleared"],
    ];
    for args in mutating {
        let refusal = refused(&fx.run(args));
        assert!(
            refusal.contains("require immediate human namespace; use")
                && refusal.contains(" human "),
            "{args:?}: {refusal}"
        );
    }
    let listed = ok(&fx.run(&["peer", "list"]));
    assert!(
        listed.starts_with("peers\n") && listed.ends_with("\nempty\n"),
        "a refused add stored nothing: {listed}"
    );
    // The reads need no operator.
    for args in [
        &["sync", "status"][..],
        &["sync", "verify"],
        &["sync", "unapplied"],
        &["sync", "unapplied", "list"],
        &["peer", "list"],
    ] {
        ok(&fx.run(args));
    }
}

// Kills: a status that is not compact rows (JSON, tables), one without the
// own log or the store line, remedies on a healthy store, and a `--log`
// filter that ignores its node.
#[test]
fn sync_status_compact_rows() {
    let fx = Fixture::new("sync-cli-status");
    let status = ok(&fx.run(&["sync", "status"]));
    let lines: Vec<_> = status.lines().collect();
    assert_eq!(lines[0], "sync_status", "{status}");
    assert!(lines[1].starts_with("discovery "), "{status}");
    let own = lines.iter().find(|l| l.starts_with("log ")).unwrap();
    assert!(own.contains(" self counter=1 head="), "{status}");
    assert!(own.split(' ').nth(1).unwrap().contains('/'), "{status}");
    assert!(
        own.contains(" applied=1 parked=0+0 failed=0 frozen=- lag=- skew_ms=0"),
        "{status}"
    );
    assert!(
        lines.iter().any(|l| l.starts_with("store bytes=")),
        "{status}"
    );
    assert!(!status.contains("remedy:"), "healthy: {status}");
    assert!(!status.contains('{'), "no JSON blobs: {status}");
    assert!(
        lines.iter().all(|l| !l.starts_with(' ')),
        "one row per item, no continuation lines: {status}"
    );

    let other = ok(&fx.run(&["sync", "status", "--log", &"ab".repeat(16)]));
    assert!(
        !other.contains("\nlog "),
        "another node has no rows: {other}"
    );
    let unknown = refused(&fx.run(&["sync", "status", "--log", "Zz9"]));
    assert!(unknown.contains("unknown node"), "{unknown}");
}

fn seed_open_evidence(fx: &Fixture) {
    let db = fx.db();
    db.execute(
        "INSERT INTO sync_clone_evidence(subject,kind,log_ix,detail,detected_at,last_seen_at) \
         SELECT n.id,'lineage_branch',n.active_log_ix,'seeded',1000,2000 FROM local_node n",
        [],
    )
    .unwrap();
}

// Kills: an open lineage-branch row without both remedy lines, evidence
// shown with no kind, a clearance that leaves the row open, one that does not
// record the operator, and a count that is not the rows cleared.
#[test]
fn sync_status_open_lineage_branch_prints_both_remedies_then_clone_cleared_clears() {
    let fx = Fixture::new("sync-cli-clone");
    seed_open_evidence(&fx);
    let status = ok(&fx.run(&["sync", "status"]));
    let remedies: Vec<_> = status
        .lines()
        .filter(|l| l.starts_with("remedy: "))
        .collect();
    assert_eq!(remedies.len(), 2, "{status}");
    assert!(
        remedies[0].contains("two live copies: move one copy's <state_dir>/node/ aside"),
        "{status}"
    );
    assert!(remedies[0].contains("sync clone-cleared"), "{status}");
    assert!(remedies[0].contains("peer add"), "{status}");
    assert!(
        remedies[1].contains("one restored copy: run `herdr-threads sync clone-cleared`"),
        "{status}"
    );
    assert!(
        status.lines().any(
            |l| l.starts_with("clone_evidence 1 lineage_branch subject=")
                && l.ends_with(": seeded")
        ),
        "{status}"
    );

    assert_eq!(
        ok(&fx.human(&["sync", "clone-cleared"])),
        "clone_cleared 1\n"
    );
    let by: String = fx
        .db()
        .query_row("SELECT cleared_by FROM sync_clone_evidence", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(by.starts_with("operator:local-user:"), "{by}");
    let after = ok(&fx.run(&["sync", "status"]));
    assert!(
        !after.contains("remedy:") && !after.contains("clone_evidence"),
        "{after}"
    );
    assert_eq!(
        ok(&fx.human(&["sync", "clone-cleared"])),
        "clone_cleared 0\n",
        "nothing left to clear"
    );
}

fn seed_foreign_log(fx: &Fixture, node: &str, entries: &[(&str, Option<&str>)]) -> i64 {
    let db = fx.db();
    db.execute(
        "INSERT INTO sync_logs(node_uuid,incarnation,is_self,pubkey,genesis_hash,contiguous_counter, \
         head_hash,applied_through,state,first_seen_at) \
         VALUES (?1,1,0,zeroblob(32),zeroblob(32),?2,?3,0,'active',0)",
        rusqlite::params![node.repeat(16), entries.len() as i64, vec![entries.len() as u8; 32]],
    )
    .unwrap();
    let log_ix = db.last_insert_rowid();
    for (index, (state, detail)) in entries.iter().enumerate() {
        let n = index as i64 + 1;
        db.execute(
            "INSERT INTO journal(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps,needs, \
             payload,signature,ingested_at,apply_state,parked_on,apply_error,raw) \
             VALUES (?1,?2,?3,?4,1,'unknown.kind',1,x'0000','',x'7b7d',zeroblob(64),0,?5,?6,?7,x'48544a524e4c3031000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000')",
            rusqlite::params![
                log_ix,
                n,
                vec![(n - 1) as u8; 32],
                vec![n as u8; 32],
                state,
                (*state == "parked_dep").then_some(*detail).flatten(),
                (*state == "failed").then_some(*detail).flatten(),
            ],
        )
        .unwrap();
    }
    log_ix
}

// Kills: a verify that flags a healthy store, and one that misses a seeded
// violation or prints it as something other than a row.
#[test]
fn sync_verify_healthy_and_seeded_violation() {
    let fx = Fixture::new("sync-cli-verify");
    assert_eq!(ok(&fx.run(&["sync", "verify"])), "verify ok\n");
    let log_ix = seed_foreign_log(&fx, "cc", &[("failed", Some("x"))]);
    // I2: a forked row at the freeze.
    let db = fx.db();
    db.execute(
        "UPDATE sync_logs SET state='frozen',frozen_at_counter=1 WHERE log_ix=?1",
        [log_ix],
    )
    .unwrap();
    db.execute(
        "INSERT INTO journal_forked(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps,needs, \
         payload,signature,received_at,raw) \
         VALUES (?1,1,zeroblob(32),x'0909090909090909090909090909090909090909090909090909090909090909',1,'unknown.kind',1,x'','',x'7b7d',zeroblob(64),0,x'48544a524e4c3031000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000')",
        [log_ix],
    )
    .unwrap();
    drop(db);
    let verify = ok(&fx.run(&["sync", "verify"]));
    assert!(
        verify.starts_with("verify violations=2\nviolation I2 cccccccc/1 counter=1: "),
        "{verify}"
    );
    assert!(
        verify.contains("\nviolation J3 cccccccc/1 counter=1: hash does not match"),
        "{verify}"
    );
}

// Kills: a verify that checks only linkage and so passes an entry whose
// stored hash is not the hash of its canonical bytes.
#[test]
fn sync_verify_reports_a_tampered_entry() {
    let fx = Fixture::new("sync-cli-tamper");
    seed_foreign_log(&fx, "dd", &[("failed", Some("x"))]);
    assert_eq!(
        ok(&fx.run(&["sync", "verify"])),
        "verify violations=1\nviolation J3 dddddddd/1 counter=1: hash does not match the entry's canonical bytes\n"
    );
}

// Kills: a peer list that hides a target whose remote lacks the binary.
// Kills: a dial that exits 127 and is still shown as a plain candidate.
#[test]
fn peer_list_shows_a_not_installed_target() {
    let fx = Fixture::with_settings(
        "sync-cli-notinst",
        r#"{"sync":{"dial_command":"exit 127 # {dest}"}}"#,
    );
    ok(&fx.human(&["peer", "add", "ghost"]));
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let listed = ok(&fx.run(&["peer", "list"]));
        if let Some(row) = listed
            .lines()
            .find(|l| l.starts_with("peer ") && l.contains(" not_installed "))
        {
            assert!(row.ends_with(": ghost"), "{listed}");
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "never not_installed: {listed}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

// Kills: a list that omits a parked or failed entry, one without the
// per-state totals, a replay that silently does nothing (nothing attempted),
// and one that claims to apply an entry whose kind is still unknown.
#[test]
fn sync_unapplied_list_and_replay() {
    let fx = Fixture::new("sync-cli-unapplied");
    assert_eq!(
        ok(&fx.run(&["sync", "unapplied"])),
        "unapplied parked_unknown=0 parked_dep=0 failed=0\n"
    );
    seed_foreign_log(
        &fx,
        "dd",
        &[
            ("parked_unknown", None),
            ("parked_dep", Some("e:thread-9")),
            ("failed", Some("boom")),
        ],
    );
    let listed = ok(&fx.run(&["sync", "unapplied", "list"]));
    let lines: Vec<_> = listed.lines().collect();
    assert_eq!(
        lines[0], "unapplied parked_unknown=1 parked_dep=1 failed=1",
        "{listed}"
    );
    assert_eq!(lines.len(), 4, "{listed}");
    assert!(
        lines[1].ends_with(" dddddddd/1 #1 unknown.kind parked_unknown"),
        "{listed}"
    );
    assert!(
        lines[2].ends_with(" dddddddd/1 #2 unknown.kind parked_dep: on e:thread-9"),
        "{listed}"
    );
    assert!(
        lines[3].ends_with(" dddddddd/1 #3 unknown.kind failed: boom"),
        "{listed}"
    );

    let paged = ok(&fx.run(&["sync", "unapplied", "list", "--limit", "2"]));
    let next = paged
        .lines()
        .find_map(|l| l.strip_prefix("next: "))
        .unwrap_or_else(|| panic!("a continuation: {paged}"));
    assert!(next.contains("sync unapplied list --after"), "{next}");

    let replayed = ok(&fx.human(&["sync", "unapplied", "replay"]));
    assert_eq!(replayed, "replayed attempted=3 applied=0 more=no\n");
    let after = ok(&fx.run(&["sync", "unapplied"]));
    assert!(
        after.starts_with("unapplied parked_unknown=3 parked_dep=0 failed=0\n"),
        "re-attempted entries of an unknown kind all park as unknown: {after}"
    );
}

// Kills: a resync that stops the daemon (or writes a marker) for an unknown
// node or one with no path.
#[test]
fn sync_resync_no_path_refused() {
    let fx = Fixture::new("sync-cli-resync");
    let unknown = refused(&fx.human(&["sync", "resync", "--from", "Zz9"]));
    assert!(unknown.contains("unknown node"), "{unknown}");

    fx.db()
        .execute(
            "INSERT INTO node_directory(node_uuid,tag,tag_k,first_seen_at) VALUES (?1,'Pq7',0,0)",
            [&"ee".repeat(16)],
        )
        .unwrap();
    let no_path = refused(&fx.human(&["sync", "resync", "--from", "Pq7"]));
    assert!(no_path.contains("no path to Pq7"), "{no_path}");
    assert!(
        !marker(&fx.state).exists(),
        "a refusal writes no resync marker"
    );
    // The daemon is still serving.
    let listed = ok(&fx.run(&["peer", "list"]));
    assert!(listed.starts_with("peers\n"), "{listed}");
    // `empty` only when there is neither a target nor a known node.
    assert!(listed.lines().all(|l| l != "empty"), "{listed}");
    assert!(
        listed.lines().all(|l| !l.starts_with("peer ")),
        "no target rows: {listed}"
    );
    // The known node is listed by node status, never heard from (ht-xg88).
    assert!(
        listed.ends_with("node Pq7 unreachable never v=- rx=- tx=-\n"),
        "{listed}"
    );
}

fn marker(state: &Path) -> PathBuf {
    state.join("node").join("resync.json")
}
