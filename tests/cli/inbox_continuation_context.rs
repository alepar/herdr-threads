//! Execute the printed argv without repairing its caller, format or routing.
use crate::lazy_config_smoke::World;
use herdr_threads::test_support::spawn;
use serde_json::Value;

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
            assert_eq!(
                w.projection_snapshot(),
                before,
                "readonly page {pages} mutated canonical state"
            );
            assert_eq!(
                w.intents(),
                intents,
                "readonly page {pages} wrote display proof/intent"
            );
        }
        let Some(argv) = next else { break };
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
