//! Execute the printed argv without repairing its caller, format or routing.
use crate::lazy_config_smoke::World;
use herdr_threads::test_support::spawn;
use serde_json::Value;

fn wait_for_send_attention(w: &World) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let unfinished: i64 = w
            .db()
            .query_row(
                "SELECT count(*) FROM work_jobs WHERE kind='send_attention' AND status!='complete'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        if unfinished == 0 {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "private send attention failed to finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn continuation_legacy_own_text_survives_v2_capability_upgrade() {
    use std::sync::atomic::Ordering;
    let w = World::new();
    let lazy = w.send("legacy traversal must leave lazy pending", &[]);
    let body = "native-legacy-é界-".repeat(1100);
    let ordinary = w.send(&body, &["--nudge", "--require-ack", &w.seats[1]]);
    wait_for_send_attention(&w);
    let proxy = crate::lazy_config_smoke::Proxy::new(w.paths(), &w.root);
    // Only the advertised capabilities change; every inbox page comes from
    // the real daemon and the producer prints its actual v1 batch cursor.
    proxy.mode.store(1, Ordering::SeqCst);
    let mut out = w.text(1, false, &["inbox", "--max-bytes", "4096"]);
    let mut recovered = String::new();
    let mut pages = 0;
    loop {
        assert!(out.len() <= 4096);
        let mut current = None;
        for line in out.lines() {
            if let Some(header) = line.strip_prefix("message ") {
                current = header.split_whitespace().next();
            } else if let Some(chunk) = line.strip_prefix("  ") {
                if current == Some(ordinary.as_str()) && !chunk.starts_with("read: ") {
                    recovered.push_str(chunk);
                }
            } else {
                current = None;
            }
        }
        let Some(next) = out.lines().find_map(|line| line.strip_prefix("next: ")) else {
            break;
        };
        let argv = shlex::split(next).unwrap();
        assert_eq!(argv[0], "herdr-threads");
        assert!(
            !argv
                .iter()
                .any(|a| matches!(a.as_str(), "human" | "--seat" | "--json" | "--machine"))
        );
        let cursor = argv.iter().position(|a| a == "--cursor").unwrap();
        assert!(
            argv[cursor + 1].starts_with("c3:"),
            "real v1 batch cursor required: {argv:?}"
        );
        for (flag, expected) in [
            (
                "--state-dir",
                w.root.join("state").to_str().unwrap().to_owned(),
            ),
            (
                "--host-endpoint",
                w.root.join("h.sock").to_str().unwrap().to_owned(),
            ),
            ("--limit", "20".into()),
            ("--max-bytes", "4096".into()),
        ] {
            let at = argv.iter().position(|a| a == flag).unwrap();
            assert_eq!(argv[at + 1], expected);
            assert_eq!(argv.iter().filter(|a| *a == flag).count(), 1);
        }
        let state: String = w.db().query_row("SELECT state FROM receipt_state WHERE message_id=?1 AND seat_id=?2 UNION SELECT state FROM receipts WHERE message_id=?1 AND seat_id=?2", [&ordinary, &w.seats[1]], |r| r.get(0)).unwrap();
        assert_eq!(state, "pending", "partial body must not ACK");
        proxy.mode.store(0, Ordering::SeqCst);
        let output = spawn::command(env!("CARGO_BIN_EXE_herdr-threads"))
            .args(&argv[1..])
            .env("HOME", w.root.join("home"))
            .env("CLAUDE_CONFIG_DIR", w.root.join("claude"))
            .env("CODEX_HOME", w.root.join("codex"))
            .env("HERDR_THREADS_OFFLINE", "1")
            .env("NO_COLOR", "1")
            .env("HERDR_PANE_ID", "w1:p2")
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SOCKET_PATH")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "upgraded daemon rejected printed v1 batch argv: {argv:?}\nstdout={} stderr={}\nrequests={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            proxy.requests()
        );
        out = String::from_utf8(output.stdout).unwrap();
        pages += 1;
        assert!(pages < 20, "legacy body cursor failed to advance");
    }
    assert!(pages > 1, "complete multi-page body required");
    assert_eq!(recovered, body);
    let state: String = w.db().query_row("SELECT state FROM receipt_state WHERE message_id=?1 AND seat_id=?2 UNION SELECT state FROM receipts WHERE message_id=?1 AND seat_id=?2", [&ordinary, &w.seats[1]], |r| r.get(0)).unwrap();
    assert_eq!(state, "acked");
    assert_eq!(w.state(&lazy, 1), "pending");
    let requests = proxy.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r["command"]["kind"] == "inbox_batch")
            .count(),
        pages + 1
    );
    assert!(!requests.iter().any(|r| matches!(
        r["command"]["kind"].as_str(),
        Some("inbox_batch_v2" | "complete_inbox_delivery")
    )));
    let acks: Vec<_> = requests
        .iter()
        .filter(|r| r["command"]["kind"] == "ack_displayed")
        .collect();
    assert_eq!(acks.len(), 1);
    assert_eq!(
        acks[0]["command"]["args"]["messages"],
        serde_json::json!([ordinary])
    );
    let claim = &acks[0]["command"]["args"]["claim"];
    assert_eq!(claim["seat"], w.seats[1]);
    assert_eq!(claim["harness"], "codex");
    assert_eq!(claim["role"], "top_level");
    assert_eq!(claim["target"], "w1:p2");
}

#[test]
fn continuation_legacy_checkin_replays_on_v2_daemon_readonly() {
    let w = World::new();
    let lazy = w.send("pending lazy delivery", &[]);
    let ordinary = w.send("pending ordinary receipt", &["--require-ack", &w.seats[1]]);
    let mut expected = std::collections::BTreeSet::from([w.thread.clone()]);
    // The real check-in builder caps its v1 inbox offer at 20 threads.
    for n in 0..21 {
        let topic = format!("legacy continuation {n}");
        let thread = w.ok(Some(0), false, &["thread", "create", "--topic", &topic])["data"]
            .as_str()
            .unwrap()
            .to_owned();
        w.ok(Some(0), false, &["invite", &thread, "--seat", &w.seats[1]]);
        expected.insert(thread);
    }
    let proxy = crate::lazy_config_smoke::Proxy::new(w.paths(), &w.root);
    let offer = w.ok(Some(1), false, &["check-in"]);
    let mut page = offer["data"]["inbox"].clone();
    assert_eq!(page["items"].as_array().unwrap().len(), 20);
    let legacy_cursor = page["next_cursor"].as_str().unwrap();
    assert!(legacy_cursor.starts_with("c3"));
    assert_eq!(
        herdr_threads::protocol::pagination::InboxBatchV2CursorState::decode(legacy_cursor),
        Err("not a v2 cursor")
    );
    wait_for_send_attention(&w);
    let before = w.projection_snapshot();
    let intents = w.intents();
    let mut recovered = std::collections::BTreeSet::new();
    let mut pages = 0;
    loop {
        for item in page["items"].as_array().unwrap() {
            assert!(recovered.insert(item["thread"].as_str().unwrap().to_owned()));
        }
        let Some(next) = page["next_argv"].as_array() else {
            break;
        };
        let argv: Vec<_> = next
            .iter()
            .map(|a| a.as_str().unwrap().to_owned())
            .collect();
        assert_eq!(argv[0], "herdr-threads");
        assert!(!argv.iter().any(|a| a == "human"));
        assert!(argv.iter().any(|a| a == "--json"));
        for (flag, expected) in [
            (
                "--state-dir",
                w.root.join("state").to_str().unwrap().to_owned(),
            ),
            (
                "--host-endpoint",
                w.root.join("h.sock").to_str().unwrap().to_owned(),
            ),
            ("--seat", w.seats[1].clone()),
            ("--max-bytes", "8000".into()),
        ] {
            let at = argv
                .iter()
                .position(|a| a == flag)
                .unwrap_or_else(|| panic!("{flag} not retained: {argv:?}"));
            assert_eq!(argv[at + 1], expected);
            assert_eq!(argv.iter().filter(|a| *a == flag).count(), 1);
        }
        // The check-in continuation omits the default limit; later inbox
        // pages may spell it out, but must retain the same effective bound.
        if let Some(at) = argv.iter().position(|a| a == "--limit") {
            assert_eq!(argv[at + 1], "20");
        }
        // Execute the printed argv unchanged, with a different caller pane and
        // no routing environment: its explicit seat must keep the read passive.
        let output = spawn::command(env!("CARGO_BIN_EXE_herdr-threads"))
            .args(&argv[1..])
            .env("HOME", w.root.join("home"))
            .env("CLAUDE_CONFIG_DIR", w.root.join("claude"))
            .env("CODEX_HOME", w.root.join("codex"))
            .env("HERDR_THREADS_OFFLINE", "1")
            .env("NO_COLOR", "1")
            .env("HERDR_PANE_ID", "w1:p1")
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SOCKET_PATH")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "printed legacy argv failed: {argv:?}\nstdout={} stderr={}\nrequests={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            proxy.requests()
        );
        assert!(output.stdout.len() <= 8000);
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["result"]["kind"], "inbox");
        page = result["result"]["data"].clone();
        assert_eq!(w.projection_snapshot(), before);
        assert_eq!(w.intents(), intents);
        pages += 1;
        assert!(pages < 10, "legacy cursor failed to advance");
    }
    assert!(pages > 0);
    assert_eq!(recovered, expected);
    assert_eq!(w.state(&lazy, 1), "pending");
    let ack: String = w.db().query_row("SELECT state FROM receipt_state WHERE message_id=?1 AND seat_id=?2 UNION SELECT state FROM receipts WHERE message_id=?1 AND seat_id=?2", [&ordinary, &w.seats[1]], |r| r.get(0)).unwrap();
    assert_eq!(ack, "pending");
    let requests = proxy.requests();
    assert!(requests.iter().any(|r| r["command"]["kind"] == "inbox"));
    assert!(!requests.iter().any(|r| matches!(
        r["command"]["kind"].as_str(),
        Some("complete_inbox_delivery" | "ack_displayed")
    )));
}

#[test]
fn continuation_malformed_namespace_is_refused() {
    let w = World::new();
    let lazy = w.send("must remain pending", &[]);
    let before = w.projection_snapshot();
    let intents = w.intents();
    for cursor in ["c3broken", "ib2broken", "unknown:broken"] {
        let output = w.raw(
            Some(1),
            false,
            &["inbox", "--seat", &w.seats[1], "--cursor", cursor],
        );
        assert!(
            !output.status.success(),
            "malformed cursor restarted inbox: {cursor}"
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("cursor"));
        assert_eq!(w.projection_snapshot(), before);
        assert_eq!(w.intents(), intents);
    }
    assert_eq!(w.state(&lazy, 1), "pending");
}

fn traverse(w: &World, who: usize, human: bool, args: &[&str], readonly: bool, selected: usize) {
    let paths = w.paths();
    let instance = herdr_threads::daemon::ownership::read_existing_namespace(&paths)
        .unwrap()
        .unwrap();
    let descriptor = herdr_threads::daemon::ownership::read_descriptor(&paths, instance).unwrap();
    capture(
        w,
        serde_json::json!({"owned_root":w.root,"daemon_pid":descriptor.pid}),
    );
    let body = "é界-'$`-".repeat(1500);
    let lazy = w.send(&body, &[]);
    let receipt_seat = if selected == 2 { 1 } else { selected };
    let ordinary_body = "ordinary-é界-'$`-".repeat(1100);
    let ordinary = w.send(&ordinary_body, &["--require-ack", &w.seats[receipt_seat]]);
    // Ordinary send returns before its worker materializes receipt projections.
    // Let this private fixture's job finish before comparing read-only pages.
    wait_for_send_attention(w);
    let pending: String = w.db().query_row("SELECT state FROM receipt_state WHERE message_id=?1 AND seat_id=?2 UNION SELECT state FROM receipts WHERE message_id=?1 AND seat_id=?2", [&ordinary, &w.seats[receipt_seat]], |r| r.get(0)).unwrap();
    assert_eq!(
        pending, "pending",
        "ordinary attention must remain actionable"
    );
    let proxy = crate::lazy_config_smoke::Proxy::new(w.paths(), &w.root);
    let before = w.projection_snapshot();
    let intents = w.intents();
    // Initial routing comes from the environment. Printed continuations must
    // pin the resolved target so replay no longer depends on those variables.
    let mut initial = spawn::command(env!("CARGO_BIN_EXE_herdr-threads"));
    if human {
        initial.arg("human");
    }
    let o = initial
        .args(args)
        .env("HERDR_PLUGIN_STATE_DIR", w.root.join("state"))
        .env("HERDR_SOCKET_PATH", w.root.join("h.sock"))
        .env("HERDR_PANE_ID", format!("w1:p{}", who + 1))
        .env("HOME", w.root.join("home"))
        .env("CLAUDE_CONFIG_DIR", w.root.join("claude"))
        .env("CODEX_HOME", w.root.join("codex"))
        .env("HERDR_THREADS_OFFLINE", "1")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "initial env-routed inbox failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let mut out = String::from_utf8(o.stdout).unwrap();
    let json = args.contains(&"--json");
    let machine = args.contains(&"--machine");
    let mut recovered = String::new();
    let mut ordinary_recovered = String::new();
    let mut pages = 0;
    loop {
        assert!(
            out.len() <= 4096,
            "final encoded page exceeds bound: {}",
            out.len()
        );
        let next = if json {
            let v: Value = serde_json::from_str(&out).expect("JSON remains JSON");
            for item in v["result"]["data"]["items"].as_array().unwrap() {
                if item["message"] == lazy {
                    recovered.push_str(item["body"].as_str().unwrap());
                }
                if item["message"] == ordinary {
                    ordinary_recovered.push_str(item["body"].as_str().unwrap());
                }
            }
            v["result"]["data"]["next_argv"].as_array().map(|a| {
                a.iter()
                    .map(|s| s.as_str().unwrap().to_owned())
                    .collect::<Vec<_>>()
            })
        } else {
            let mut current = None;
            for line in out.lines() {
                if let Some(header) = line.strip_prefix("message ") {
                    current = header.split_whitespace().next();
                } else if let Some(body_line) = line.strip_prefix("  ") {
                    if body_line.starts_with("read: ") {
                        continue;
                    }
                    if current == Some(lazy.as_str()) {
                        recovered.push_str(body_line);
                    }
                    if current == Some(ordinary.as_str()) {
                        ordinary_recovered.push_str(body_line);
                    }
                } else {
                    current = None;
                }
            }
            out.lines()
                .find_map(|l| l.strip_prefix("next: "))
                .map(|s| shlex::split(s).unwrap())
        };
        if readonly {
            let after = w.projection_snapshot();
            let changed: Vec<_> = after
                .iter()
                .zip(&before)
                .filter(|(after, before)| after != before)
                .collect();
            assert!(
                changed.is_empty(),
                "readonly page {pages} mutated canonical tables: {changed:?}"
            );
            assert_eq!(
                w.intents(),
                intents,
                "readonly page {pages} wrote display proof/intent"
            );
        }
        let Some(argv) = next else { break };
        let cursor = argv.iter().position(|a| a == "--cursor").unwrap();
        assert!(
            argv[cursor + 1].starts_with("ib2"),
            "v2 continuation lost: {argv:?}"
        );
        assert_eq!(argv[0], "herdr-threads");
        assert_eq!(
            argv.get(1).map(String::as_str) == Some("human"),
            human,
            "actor lost: {argv:?}"
        );
        for (flag, expected) in [
            ("--state-dir", w.root.join("state")),
            ("--host-endpoint", w.root.join("h.sock")),
        ] {
            let at = argv
                .iter()
                .position(|s| s == flag)
                .expect("routing retained");
            assert_eq!(argv[at + 1], expected.to_str().unwrap());
            assert_eq!(argv.iter().filter(|s| *s == flag).count(), 1);
        }
        assert_eq!(
            argv.iter().any(|a| a == "--json"),
            json,
            "format lost: {argv:?}"
        );
        assert_eq!(
            argv.iter().any(|a| a == "--machine"),
            machine,
            "machine selector lost: {argv:?}"
        );
        if readonly {
            let at = argv
                .iter()
                .position(|a| a == "--seat")
                .expect("readonly selected seat retained");
            assert_eq!(argv[at + 1], w.seats[selected]);
        } else {
            assert!(
                !argv.iter().any(|a| a == "--seat"),
                "own default text lost: {argv:?}"
            );
        }
        let o = spawn::command(env!("CARGO_BIN_EXE_herdr-threads"))
            .args(&argv[1..])
            .env("HOME", w.root.join("home"))
            .env("CLAUDE_CONFIG_DIR", w.root.join("claude"))
            .env("CODEX_HOME", w.root.join("codex"))
            .env("HERDR_THREADS_OFFLINE", "1")
            .env("NO_COLOR", "1")
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SOCKET_PATH")
            .env("HERDR_PANE_ID", format!("w1:p{}", who + 1))
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "printed argv failed: {argv:?}\nstdout={} stderr={}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        capture(
            w,
            serde_json::json!({"continuation_argv":argv,"status":o.status.code(),"stdout":String::from_utf8_lossy(&o.stdout),"stderr":String::from_utf8_lossy(&o.stderr)}),
        );
        out = String::from_utf8(o.stdout).unwrap();
        pages += 1;
        assert!(pages < 60, "cursor failed to advance");
    }
    assert!(pages > 1, "multi-page UTF-8 chain required");
    assert_eq!(recovered, body);
    assert_eq!(
        ordinary_recovered,
        if selected == receipt_seat {
            ordinary_body
        } else {
            String::new()
        }
    );
    assert_eq!(
        w.state(&lazy, selected),
        if readonly { "pending" } else { "displayed" }
    );
    let ack: String = w.db().query_row("SELECT state FROM receipt_state WHERE message_id=?1 AND seat_id=?2 UNION SELECT state FROM receipts WHERE message_id=?1 AND seat_id=?2", [&ordinary, &w.seats[receipt_seat]], |r| r.get(0)).unwrap();
    assert_eq!(
        ack,
        if readonly || human {
            "pending"
        } else {
            "acked"
        }
    );
    let mutations: Vec<_> = proxy
        .requests()
        .into_iter()
        .filter(|v| {
            matches!(
                v["command"]["kind"].as_str(),
                Some("complete_inbox_delivery" | "ack_displayed")
            )
        })
        .collect();
    let inbox_requests: Vec<_> = proxy
        .requests()
        .into_iter()
        .filter(|r| {
            matches!(
                r["command"]["kind"].as_str(),
                Some("inbox" | "inbox_batch_v2")
            )
        })
        .collect();
    assert_eq!(inbox_requests[0]["command"]["kind"], "inbox_batch_v2");
    assert!(inbox_requests[0]["command"]["args"]["page"]["cursor"].is_null());
    assert!(
        inbox_requests
            .iter()
            .skip(1)
            .all(|r| r["command"]["kind"] == "inbox_batch_v2")
    );
    capture(
        w,
        serde_json::json!({"accountable_requests":mutations,"pages":pages}),
    );
    if readonly {
        assert!(
            mutations.is_empty(),
            "passive traversal submitted mutation: {mutations:?}"
        );
    } else {
        let complete: Vec<_> = mutations
            .iter()
            .filter(|v| v["command"]["kind"] == "complete_inbox_delivery")
            .collect();
        assert_eq!(complete.len(), 1);
        assert_eq!(
            complete[0]["command"]["args"]["messages"],
            serde_json::json!([lazy])
        );
        for mutation in &mutations {
            let claim = &mutation["command"]["args"]["claim"];
            assert_eq!(claim["seat"], w.seats[selected]);
            assert_eq!(claim["harness"], if human { "human" } else { "codex" });
            assert_eq!(claim["role"], "top_level");
            assert_eq!(claim["target"], format!("w1:p{}", who + 1));
        }
        assert_eq!(
            mutations
                .iter()
                .filter(|v| v["command"]["kind"] == "ack_displayed")
                .count(),
            usize::from(!human)
        );
    }
}

#[test]
fn continuation_machine_preserves_readonly_across_utf8_chunks() {
    let w = World::new();
    traverse(
        &w,
        1,
        false,
        &["inbox", "--machine", "--max-bytes", "4096"],
        true,
        1,
    );
}
#[test]
fn continuation_json_foreign_seat_preserves_scope() {
    let w = World::new();
    traverse(
        &w,
        1,
        false,
        &[
            "inbox",
            "--json",
            "--seat",
            &w.seats[2],
            "--max-bytes",
            "4096",
        ],
        true,
        2,
    );
}
#[test]
fn continuation_explicit_own_and_foreign_text_stay_readonly() {
    for selected in [1, 2] {
        let w = World::new();
        traverse(
            &w,
            1,
            false,
            &["inbox", "--seat", &w.seats[selected], "--max-bytes", "4096"],
            true,
            selected,
        );
    }
}
#[test]
fn continuation_human_own_text_keeps_actor_and_completes() {
    let w = World::new();
    traverse(&w, 2, true, &["inbox", "--max-bytes", "4096"], false, 2);
}
#[test]
fn continuation_agent_own_text_retains_ack_and_completion() {
    let w = World::new();
    traverse(&w, 1, false, &["inbox", "--max-bytes", "4096"], false, 1);
}

fn capture(w: &World, value: Value) {
    use std::io::Write;
    if let Some(dir) = std::env::var_os("HT_LAZY_SMOKE_CAPTURE") {
        let path = std::path::PathBuf::from(dir).join(format!(
            "{}-continuations.jsonl",
            w.root.file_name().unwrap().to_string_lossy()
        ));
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(file, "{value}").unwrap();
    }
}

/// Catches fitting the legacy Human table before its topic column is installed.
#[test]
fn continuation_legacy_human_long_topics_fit_and_remain_readonly() {
    use std::sync::atomic::Ordering;
    let w = World::new();
    let ordinary = w.send(
        "pending control",
        &["--nudge", "--require-ack", &w.seats[1]],
    );
    let lazy = w.send("passive control", &[]);
    let mut expected = std::collections::BTreeSet::from([w.thread.clone()]);
    for n in 0..21 {
        let topic = format!("topic-{n:02}-{}", "界".repeat(45));
        let thread = w.ok(Some(0), false, &["thread", "create", "--topic", &topic])["data"]
            .as_str()
            .unwrap()
            .to_owned();
        w.ok(Some(0), false, &["invite", &thread, "--seat", &w.seats[1]]);
        expected.insert(thread);
    }
    wait_for_send_attention(&w);
    let proxy = crate::lazy_config_smoke::Proxy::new(w.paths(), &w.root);
    proxy.mode.store(1, Ordering::SeqCst);
    let before = w.projection_snapshot();
    let intents = w.intents();
    let mut out = w.text(
        2,
        true,
        &[
            "inbox",
            "--human",
            "--seat",
            &w.seats[1],
            "--max-bytes",
            "2400",
        ],
    );
    let mut recovered = std::collections::BTreeSet::new();
    let mut pages = 0;
    loop {
        assert!(
            out.len() <= 2400,
            "actual Human bytes exceeded bound: {}",
            out.len()
        );
        assert!(out.contains("TOPIC"), "{out}");
        for line in out.lines() {
            if let Some(id) = line
                .split_whitespace()
                .next()
                .filter(|id| expected.contains(*id))
            {
                assert!(recovered.insert(id.to_owned()), "duplicate thread {id}");
            }
        }
        assert_eq!(w.projection_snapshot(), before);
        assert_eq!(w.intents(), intents);
        let Some(next) = out.lines().find_map(|line| line.strip_prefix("more: ")) else {
            break;
        };
        let argv = shlex::split(next).unwrap();
        assert_eq!(argv[0], "herdr-threads");
        assert_eq!(argv[1], "human");
        assert!(argv.iter().any(|a| a == "--human"));
        let cursor = argv.iter().position(|a| a == "--cursor").unwrap();
        assert!(argv[cursor + 1].starts_with("c3:"), "{argv:?}");
        for (flag, expected) in [
            ("--seat", w.seats[1].clone()),
            ("--max-bytes", "2400".into()),
            ("--state-dir", w.root.join("state").to_str().unwrap().into()),
            (
                "--host-endpoint",
                w.root.join("h.sock").to_str().unwrap().into(),
            ),
        ] {
            let at = argv.iter().position(|a| a == flag).unwrap();
            assert_eq!(argv[at + 1], expected);
        }
        // A capability upgrade cannot change the protocol of the captured page.
        proxy.mode.store(0, Ordering::SeqCst);
        let output = spawn::command(env!("CARGO_BIN_EXE_herdr-threads"))
            .args(&argv[1..])
            .env("HOME", w.root.join("home"))
            .env("CLAUDE_CONFIG_DIR", w.root.join("claude"))
            .env("CODEX_HOME", w.root.join("codex"))
            .env("HERDR_THREADS_OFFLINE", "1")
            .env("NO_COLOR", "1")
            .env("HERDR_PANE_ID", "w1:p3")
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SOCKET_PATH")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "printed continuation {argv:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        out = String::from_utf8(output.stdout).unwrap();
        pages += 1;
        assert!(pages < 20, "cursor failed to advance");
    }
    assert!(pages > 0, "long topic page must continue");
    assert_eq!(recovered, expected);
    assert_eq!(w.state(&lazy, 1), "pending");
    let ack: String = w.db().query_row("SELECT state FROM receipt_state WHERE message_id=?1 AND seat_id=?2 UNION SELECT state FROM receipts WHERE message_id=?1 AND seat_id=?2", [&ordinary, &w.seats[1]], |r| r.get(0)).unwrap();
    assert_eq!(ack, "pending");
    let requests = proxy.requests();
    assert!(requests.iter().any(|r| r["command"]["kind"] == "directory"));
    assert!(requests.iter().any(|r| r["command"]["kind"] == "inbox"));
    assert!(!requests.iter().any(|r| matches!(
        r["command"]["kind"].as_str(),
        Some("inbox_batch_v2" | "complete_inbox_delivery" | "ack_displayed")
    )));
}
