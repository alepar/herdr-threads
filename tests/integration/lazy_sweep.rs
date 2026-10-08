//! Cross-tree tests through the actual executable and private daemon.
use super::lazy_config_smoke::{Proxy, World};
use serde_json::Value;
use std::sync::atomic::Ordering;

fn pending_ordinary(w: &World, message: &str) -> i64 {
    w.db().query_row("SELECT count(*) FROM (SELECT message_id,seat_id FROM receipt_state WHERE message_id=?1 AND seat_id=?2 AND state='pending' UNION SELECT message_id,seat_id FROM receipts WHERE message_id=?1 AND seat_id=?2 AND state='pending')", [message, &w.seats[1]], |r| r.get(0)).unwrap()
}
fn lazy_attention(w: &World, message: &str) -> i64 {
    w.db().query_row("SELECT (SELECT count(*) FROM receipts WHERE message_id=?1)+(SELECT count(*) FROM receipt_state WHERE message_id=?1)+(SELECT count(*) FROM work_jobs WHERE kind='send_attention' AND subject_id=?1)", [message], |r| r.get(0)).unwrap()
}
fn attention(w: &World) -> herdr_threads::protocol::attention::AttentionDigest {
    let db = w.db();
    let instance: String = db
        .query_row(
            "SELECT instance_id FROM seats WHERE id=?1",
            [&w.seats[1]],
            |r| r.get(0),
        )
        .unwrap();
    herdr_threads::store::attention::seat_digest(
        &db,
        &instance,
        &herdr_threads::protocol::ids::SeatId::new(&w.seats[1]),
        &|| Ok(()),
    )
    .unwrap()
    .digest
}
fn body_lines(text: &str) -> String {
    text.lines()
        .filter_map(|l| l.strip_prefix("  "))
        .filter(|l| !l.starts_with("read: "))
        .collect()
}
fn next(text: &str) -> Option<Vec<String>> {
    text.lines()
        .find_map(|l| l.strip_prefix("next: "))
        .map(|l| {
            let argv = shlex::split(l).unwrap();
            assert_eq!(argv[0], "herdr-threads");
            argv[1..].to_vec()
        })
}

#[test]
fn lazy_sweep_full_body_mixed_page_then_independent_retry() {
    let w = World::new();
    let body = "sweep-é-界-".repeat(950);
    let lazy = w.send(&body, &[]);
    let ordinary_body = "independent ordinary ACK";
    let ordinary = w.send(ordinary_body, &["--require-ack", &w.seats[1]]);
    let proxy = Proxy::new(w.paths(), &w.root);
    proxy.mode.store(2, Ordering::SeqCst); // completion commits; only its reply is lost
    let mut argv = vec!["inbox".to_owned(), "--max-bytes".into(), "4096".into()];
    let mut content = String::new();
    let mut errors = String::new();
    let mut pages = 0;
    let mut mixed_page = false;
    loop {
        let refs: Vec<_> = argv.iter().map(String::as_str).collect();
        let out = w.raw(Some(1), false, &refs);
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(text.len() <= 4096);
        mixed_page |= text.contains("[lazy]") && text.contains(ordinary_body);
        content.push_str(&body_lines(&text));
        if content.len() < body.len() {
            assert_eq!(w.state(&lazy, 1), "pending", "partial body cannot settle");
        }
        if !out.status.success() {
            errors.push_str(&String::from_utf8_lossy(&out.stderr));
        }
        pages += 1;
        assert!(pages < 40);
        let Some(n) = next(&text) else { break };
        argv = n;
    }
    assert!(pages > 2);
    assert!(
        mixed_page,
        "both settlement kinds must share one selected page"
    );
    assert_eq!(
        content,
        format!("{body}{ordinary_body}"),
        "exact UTF8 content in canonical lazy-first order"
    );
    assert_eq!(
        pending_ordinary(&w, &ordinary),
        0,
        "lazy reply failure must not block ACK"
    );
    let acknowledged: i64 = w.db().query_row("SELECT count(*) FROM (SELECT message_id,seat_id FROM receipt_state WHERE message_id=?1 AND seat_id=?2 AND state='acked' UNION SELECT message_id,seat_id FROM receipts WHERE message_id=?1 AND seat_id=?2 AND state='acked')", [&ordinary, &w.seats[1]], |r| r.get(0)).unwrap();
    assert_eq!(acknowledged, 1);
    assert_eq!(w.state(&lazy, 1), "displayed");
    assert_eq!(lazy_attention(&w, &lazy), 0);
    let reference = errors
        .split_whitespace()
        .find(|s| s.starts_with("local:"))
        .expect("lost reply retains exact retry ref")
        .trim_end_matches(['.', ',', ';']);
    let original = proxy
        .requests()
        .into_iter()
        .find(|q| q["command"]["kind"] == "complete_inbox_delivery")
        .unwrap()["command"]
        .clone();
    proxy.mode.store(0, Ordering::SeqCst);
    w.text(1, false, &["retry", reference]);
    let completions: Vec<_> = proxy
        .requests()
        .into_iter()
        .filter(|q| q["command"]["kind"] == "complete_inbox_delivery")
        .map(|q| q["command"].clone())
        .collect();
    assert_eq!(completions, vec![original.clone(), original]);
    assert_eq!(
        proxy
            .requests()
            .iter()
            .filter(|q| q["command"]["kind"] == "ack_displayed")
            .count(),
        1
    );
    assert_eq!(w.state(&lazy, 1), "displayed");
    assert_eq!(lazy_attention(&w, &lazy), 0);
}

#[test]
fn lazy_sweep_restart_readonly_then_text_delivery_without_attention() {
    let w = World::new();
    let digest = attention(&w);
    let lazy = w.send("restart passive sweep", &[]);
    assert_eq!(attention(&w), digest);
    let before = w.intents();
    let projection = w.projection_snapshot();
    w.ok(None, false, &["daemon", "stop"]);
    w.ok(None, false, &["daemon", "ensure"]);
    for args in [
        vec!["inbox", "--json"],
        vec!["inbox", "--machine"],
        vec!["inbox", "--seat", &w.seats[1]],
    ] {
        assert!(w.text(1, false, &args).contains("restart passive sweep"));
        assert_eq!(w.state(&lazy, 1), "pending");
        assert_eq!(w.intents(), before);
        assert_eq!(w.projection_snapshot(), projection);
    }
    let check = w.ok(
        Some(1),
        false,
        &["check-in", "--lifecycle-event", "sweep-restart"],
    );
    let data = &check["data"];
    assert_eq!(attention(&w), digest);
    assert_eq!(data["warning_count"], 0, "{check}");
    assert_eq!(data["inbox"]["items"], serde_json::json!([]), "{check}");
    assert!(!data.to_string().contains(&lazy), "{check}");
    assert_eq!(lazy_attention(&w, &lazy), 0);
    assert!(
        w.text(1, false, &["inbox"])
            .contains("restart passive sweep")
    );
    assert_eq!(w.state(&lazy, 1), "displayed");
    assert_eq!(attention(&w), digest);
    assert_eq!(lazy_attention(&w, &lazy), 0);
}

fn cache(w: &World) -> Vec<Vec<String>> {
    let db = w.db();
    let mut stmt = db
        .prepare("SELECT * FROM summary_blocks ORDER BY id")
        .unwrap();
    let n = stmt.column_count();
    stmt.query_map([], |r| {
        Ok((0..n)
            .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
            .collect())
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}
#[test]
fn lazy_sweep_canonical_markers_summary_cache_and_legacy_reads() {
    let w = World::new();
    let lazy = w.send("canonical sweep needle", &[]);
    let ordinary = w.send("ordinary sweep needle", &["--nudge"]);
    // A retained immutable derived-summary fixture makes cache equality nonvacuous.
    let writer = rusqlite::Connection::open(w.paths().database_path).unwrap();
    writer.execute("INSERT INTO summary_blocks(id,instance_id,thread_id,chunking_version,level,idx,first_seq,last_seq,source_hash,narrative,author_seat_id,model,prompt_version,created_at) SELECT 'sweep-cache',instance_id,thread_id,'sweep-v1',0,0,sequence,sequence,'fixture-hash','retained canonical narrative',actor_seat_id,'fixture','fixture',decision_at FROM messages WHERE id=?1", [&lazy]).unwrap();
    drop(writer);
    let cached = cache(&w);
    assert_eq!(cached.len(), 1);
    let before = w.intents();
    for args in [
        vec!["body", &lazy],
        vec!["read", &w.thread],
        vec!["search", "needle", "--thread", &w.thread],
    ] {
        let out = w.text(1, false, &args);
        assert!(
            out.contains("[lazy]") && out.contains("canonical sweep needle"),
            "{out}"
        );
        assert_eq!(w.state(&lazy, 1), "pending");
        assert_eq!(cache(&w), cached);
        assert_eq!(w.intents(), before);
    }
    let json: Value = serde_json::from_str(&w.text(1, false, &["body", &lazy, "--json"])).unwrap();
    assert!(!json.to_string().contains("[lazy]"));
    let proxy = Proxy::new(w.paths(), &w.root);
    proxy.mode.store(1, Ordering::SeqCst); // pre-lazy capabilities, supported wire6
    let legacy = w.text(1, false, &["body", &lazy]);
    assert!(legacy.contains("canonical sweep needle") && !legacy.contains("[lazy]"));
    let inbox = w.text(1, false, &["inbox", "--machine"]);
    assert!(inbox.contains(&ordinary) || inbox.contains(&w.thread));
    assert!(!inbox.contains(&lazy));
    assert_eq!(w.state(&lazy, 1), "pending");
    assert_eq!(cache(&w), cached);
    assert_eq!(w.intents(), before);
    assert_eq!(lazy_attention(&w, &lazy), 0);
}
