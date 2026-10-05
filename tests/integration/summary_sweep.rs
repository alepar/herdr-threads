//! Cross-flow sweep for thread summaries (ht-1ip.18): the flows no per-seam
//! case covers, on the real daemon through the installed executable. A child
//! of `summary_flow` so it shares that module's world, fixture and scripted
//! worker without widening their visibility.

use super::{CHUNKS, Fixture, SETTINGS, tickets};
use herdr_threads::{protocol::summary::SummarySettings, store::poke::due_pokes};
use serde_json::{Value, json};
use std::{fs, time::Duration};

fn now_ms() -> i64 {
    super::utc_ms() as i64
}

/// The settings the daemon runs with (the fixture's file over the defaults).
fn daemon_settings() -> SummarySettings {
    let file: Value = serde_json::from_str(SETTINGS).unwrap();
    serde_json::from_value(file["summary"].clone()).unwrap()
}

/// Messages with a poke due for `seat` at `now`, through the store's real due
/// scan on the daemon's own database.
fn poke_due_messages(fx: &Fixture, seat: &str, now: i64) -> Vec<String> {
    let db = fx.world.db();
    due_pokes(&db, now, &daemon_settings(), 16)
        .unwrap()
        .into_iter()
        .filter(|due| due.seat.as_str() == seat)
        .flat_map(|due| due.receipts)
        .map(|receipt| receipt.message.as_str().to_owned())
        .collect()
}

/// A joiner is told about the summary only when the thread holds a full chunk,
/// and following the hint reaches Work and then Ready. C joins two fresh
/// threads of A's: one below a chunk, one with a dozen ordinary messages.
#[test]
fn join_hint_then_summary() {
    let fx = Fixture::build();
    let caller_c = fx.caller_c();
    let create = |topic: &str| {
        fx.world
            .cli(
                Some(fx.caller_a()),
                None,
                &["thread", "create", "--topic", topic],
            )
            .text("thread create")
    };
    let send = |thread: &str, text: &str| {
        fx.world
            .cli(Some(fx.caller_a()), None, &["send", thread, "--body", text])
            .data("send")
    };
    let join = |thread: &str| {
        fx.world
            .cli(
                Some(fx.caller_a()),
                None,
                &["invite", thread, "--seat", &fx.c],
            )
            .data("invite");
        let accepted = fx.world.human(Some(caller_c), &["accept", thread]);
        assert_eq!(accepted.code, 0, "{}{}", accepted.stdout, accepted.stderr);
        accepted.stdout
    };

    // Below one chunk: no hint.
    let tiny = create("tiny");
    send(&tiny, "hello");
    let out = join(&tiny);
    assert!(!out.contains("summary available"), "no hint: {out}");

    // Several full chunks: the hint names the command for the thread.
    let big = create("big");
    for index in 0..12 {
        send(&big, &super::body(index));
    }
    let out = join(&big);
    // The command carries the invocation's own global flags.
    let command = format!("summary {big}");
    assert!(
        out.lines()
            .any(|line| line.starts_with("summary available: herdr-threads ")
                && line.ends_with(&command)),
        "{out}"
    );

    // Following the hint: Work, then the stored blocks make it Ready.
    let summary = || {
        fx.world
            .cli(Some(caller_c), None, &["summary", &big])
            .data("summary")
    };
    let mut status = String::new();
    for _ in 0..3 {
        let work = summary();
        status = work["status"].as_str().unwrap().to_owned();
        if status == "ready" {
            break;
        }
        for ticket in tickets(&work) {
            let bundle = fx.fetch(caller_c, &ticket);
            assert_eq!(bundle["status"], "bundle", "{bundle}");
            let submission = json!({
                "submission_schema": herdr_threads::protocol::summary::SUBMISSION_SCHEMA,
                "narrative": format!("messages #{}-#{}", ticket.first, ticket.last),
                "prompt_version": "integration-1",
                "model": "scripted",
            });
            let (code, stored) = fx.submit(caller_c, &ticket, &submission);
            assert_eq!((code, &stored["status"]), (0, &json!("stored")), "{stored}");
        }
    }
    assert_eq!(status, "ready", "following the hint ends in Ready");
}

/// Waits until the daemon's deadline worker has projected the seat's receipt
/// for `message` (the poke scan reads only projected rows).
fn wait_projected(fx: &Fixture, seat: &str, message: &str) {
    super::wait_until("receipt projection", Duration::from_secs(60), || {
        fx.world
            .db()
            .query_row(
                "SELECT count(*) FROM receipt_state WHERE seat_id=?1 AND message_id=?2",
                [seat, message],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            > 0
    });
}

/// Recovery to Ready with the hold and the poke suppression in force: while
/// B's catch-up row is active a new message is held and no soft-deadline poke
/// is due for B even inside the soft window; once Ready the held receipt is
/// pushed and is poked once past its soft point.
#[test]
fn recovery_to_ready_with_hold_and_poke_suppressed() {
    let fx = Fixture::build();
    // The worker projects one send per pass: drain the fixture's backlog so
    // the receipts below are projected well inside the entry extension.
    super::wait_until("send projections", Duration::from_secs(120), || {
        fx.world
            .db()
            .query_row(
                "SELECT count(*) FROM work_jobs WHERE status<>'complete'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            == 0
    });
    let work = fx.summary(fx.caller_b());
    let jobs = tickets(&work);
    let row = fx.world.catch_up(&fx.b, &fx.thread).expect("row");
    assert_eq!(row.state, "active", "{row:?}");

    let held = fx.send_as_a(
        "a request that arrives mid catch-up",
        &["--require-ack", &fx.b, "--deadline", "30"],
    );
    assert!(!fx.boundary_b().contains(&held), "held while catching up");
    wait_projected(&fx, &fx.b, &held);
    wait_projected(&fx, &fx.b, &fx.ack);
    assert_eq!(
        fx.world.catch_up(&fx.b, &fx.thread).unwrap().state,
        "active",
        "still catching up"
    );

    // One tick before each receipt's effective deadline is inside its soft
    // window; B is still not poked: the active row owns the receipts.
    let ack_effective = fx.world.pending_for(&fx.b, &fx.ack)["effective_deadline"]
        .as_i64()
        .expect("extended ack");
    // The 30 s receipt outlasts the entry extension: its effective deadline
    // is its own frozen one, which the projection reports as `deadline`.
    let held_effective = fx.world.pending_for(&fx.b, &held)["deadline"]
        .as_i64()
        .expect("held receipt deadline");
    for at in [ack_effective - 1, held_effective - 1] {
        assert_eq!(
            poke_due_messages(&fx, &fx.b, at),
            Vec::<String>::new(),
            "no poke for B while its catch-up row is active (at {at})"
        );
    }

    fx.run_jobs(fx.caller_b(), &jobs);
    let ready = fx.summary(fx.caller_b());
    assert_eq!(ready["status"], "ready", "{ready}");
    let row = fx.world.catch_up(&fx.b, &fx.thread).unwrap();
    assert_eq!(row.end_reason.as_deref(), Some("ready"), "{row:?}");

    // Released: pushed at the boundary, and now a poke candidate.
    assert!(fx.boundary_b().contains(&held), "pushed after Ready");
    let item = fx.world.pending_for(&fx.b, &held);
    let held_effective = item["effective_deadline"]
        .as_i64()
        .or_else(|| item["deadline"].as_i64())
        .expect("held deadline");
    assert!(
        held_effective > now_ms(),
        "the 30 s receipt is still within its deadline"
    );
    assert_eq!(
        poke_due_messages(&fx, &fx.b, held_effective - 1),
        vec![held.clone()],
        "past its soft point and before its effective deadline the held receipt is poked"
    );
    let window: i64 = fx
        .world
        .db()
        .query_row(
            "SELECT deadline_at-available_at FROM receipt_state WHERE seat_id=?1 AND message_id=?2",
            [fx.b.as_str(), held.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    let soft = held_effective - (window as f64 * (1.0 - daemon_settings().soft_fraction)) as i64;
    assert_eq!(
        poke_due_messages(&fx, &fx.b, soft + 1_000),
        vec![held.clone()],
        "just past the soft point"
    );
    assert_eq!(
        poke_due_messages(&fx, &fx.b, soft - 1_000),
        Vec::<String>::new(),
        "before its soft point it is not"
    );
}

/// A settings change that alters the chunking starts a new generation: the
/// old blocks stay stored but are never shown, and new-version jobs are
/// offered.
#[test]
fn settings_change_starts_a_new_generation() {
    let fx = Fixture::build();
    let work = fx.summary(fx.caller_b());
    let jobs = tickets(&work);
    fx.run_jobs(fx.caller_b(), &jobs);
    assert_eq!(fx.summary(fx.caller_b())["status"], "ready");
    let count = |sql: &str| -> i64 { fx.world.db().query_row(sql, [], |r| r.get(0)).unwrap() };
    let blocks_before = count("SELECT count(*) FROM summary_blocks");
    assert_eq!(blocks_before, CHUNKS.len() as i64);
    let old_version: String = fx
        .world
        .db()
        .query_row(
            "SELECT DISTINCT chunking_version FROM summary_blocks",
            [],
            |r| r.get(0),
        )
        .unwrap();

    // Restart the daemon with a different chunk size.
    fx.world.cli(None, None, &["daemon", "stop"]);
    let mut settings: Value = serde_json::from_str(SETTINGS).unwrap();
    settings["summary"]["chunk_bytes"] = json!(2048);
    let file = fx.world.instance_dir.join("settings.json");
    fs::write(&file, settings.to_string()).unwrap();
    fx.world
        .cli(None, None, &["daemon", "ensure"])
        .data("daemon ensure");

    let work = fx.summary(fx.caller_b());
    assert_eq!(work["status"], "work", "old blocks are never shown: {work}");
    let jobs = tickets(&work);
    assert!(!jobs.is_empty() && jobs.len() < CHUNKS.len(), "{work}");
    assert_eq!(
        count("SELECT count(*) FROM summary_blocks"),
        blocks_before,
        "old blocks are kept"
    );
    let versions: Vec<String> = {
        let db = fx.world.db();
        let mut statement = db
            .prepare("SELECT DISTINCT chunking_version FROM summary_jobs ORDER BY 1")
            .unwrap();
        statement
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(
        versions.len(),
        2,
        "a second generation of jobs: {versions:?}"
    );
    assert!(versions.contains(&old_version), "{versions:?}");
}
