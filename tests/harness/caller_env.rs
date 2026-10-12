use super::*;
use std::collections::HashMap;

struct FakeTable(Vec<ProcessRecord>);
impl ProcessInfo for FakeTable {
    fn process(&self, pid: u32) -> Option<ProcessRecord> {
        self.0.iter().find(|p| p.pid == pid).cloned()
    }
}

fn rec(pid: u32, parent: u32, start: u64, argv: &[&str]) -> ProcessRecord {
    ProcessRecord {
        pid,
        parent: Some(parent),
        start,
        argv: argv.iter().map(|a| a.to_string()).collect(),
    }
}

fn run(
    table: &FakeTable,
    env: &[(&str, &str)],
    agent_id: Option<&str>,
) -> Result<EnvClaim, EnvRefusal> {
    let map: HashMap<String, String> = env
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let lookup = move |name: &str| map.get(name).cloned();
    derive(&EnvInputs {
        env: &lookup,
        self_pid: 100,
        agent_id: agent_id.map(AgentId::new),
        processes: table,
    })
}

fn claude_table() -> FakeTable {
    FakeTable(vec![
        rec(100, 90, 1000, &["herdr-threads", "me"]),
        rec(90, 80, 900, &["sh"]),
        rec(80, 1, 800, &["node", "/x/@anthropic-ai/claude-code/cli.js"]),
    ])
}

const CLAUDE_ENV: &[(&str, &str)] = &[("CLAUDE_CODE_SESSION_ID", "s1"), ("CLAUDE_PID", "80")];

#[test]
fn claude_top_level_from_session_and_claude_pid() {
    let claim = run(&claude_table(), CLAUDE_ENV, None).unwrap();
    assert_eq!(claim.harness, Harness::Claude);
    assert_eq!(claim.native_session.as_str(), "s1");
    assert_eq!(claim.agent_id, None);
    assert_eq!(claim.role, CallerRole::TopLevel);
    assert_eq!(
        claim.fingerprint,
        LivenessFingerprint {
            pid: 80,
            start: 800
        }
    );
    assert_eq!(claim.provenance, "cooperative_env_session");
}

#[test]
fn claude_subagent_shares_the_parent_fingerprint() {
    // A subagent shares its parent's pid, so ht-54r.10 keys same-pid supersede on role:
    // it is never a session switch.
    let top = run(&claude_table(), CLAUDE_ENV, None).unwrap();
    let sub = run(&claude_table(), CLAUDE_ENV, Some("a1")).unwrap();
    assert_eq!(sub.role, CallerRole::Subagent);
    assert_eq!(sub.agent_id, Some(AgentId::new("a1")));
    assert_eq!(sub.provenance, "cooperative_hook_agent");
    assert_eq!(sub.fingerprint, top.fingerprint);
}

fn codex_table() -> FakeTable {
    FakeTable(vec![
        rec(100, 90, 1000, &["herdr-threads"]),
        rec(90, 80, 900, &["sh"]),
        rec(80, 1, 800, &["/usr/local/bin/codex", "--flag"]),
    ])
}

#[test]
fn codex_top_level_when_thread_equals_session() {
    let env = [("CODEX_SESSION_ID", "c1"), ("CODEX_THREAD_ID", "c1")];
    let claim = run(&codex_table(), &env, Some("ignored")).unwrap();
    assert_eq!(claim.harness, Harness::Codex);
    assert_eq!(claim.native_session.as_str(), "c1");
    assert_eq!(claim.agent_id, None);
    assert_eq!(claim.role, CallerRole::TopLevel);
    assert_eq!(claim.fingerprint.pid, 80);
    assert_eq!(claim.provenance, "cooperative_env_session");
}

#[test]
fn codex_subagent_when_thread_differs() {
    let env = [("CODEX_SESSION_ID", "c1"), ("CODEX_THREAD_ID", "t9")];
    let claim = run(&codex_table(), &env, None).unwrap();
    assert_eq!(claim.native_session.as_str(), "c1");
    assert_eq!(claim.agent_id, Some(AgentId::new("t9")));
    assert_eq!(claim.role, CallerRole::Subagent);
    assert_eq!(claim.fingerprint.pid, 80);
}

/// Kills: a malformed `CODEX_THREAD_ID` silently downgraded to a top-level claim (a subagent
/// would then act, and display, as its parent).
#[test]
fn codex_malformed_subagent_marker_is_refused_not_downgraded() {
    for bad in ["has space", &"x".repeat(129)] {
        let env = [("CODEX_SESSION_ID", "c1"), ("CODEX_THREAD_ID", bad)];
        let refusal = run(&codex_table(), &env, None).unwrap_err();
        assert_eq!(
            refusal,
            EnvRefusal::MalformedSubagentMarker {
                harness: "codex",
                var: "CODEX_THREAD_ID"
            }
        );
        assert_eq!(refusal.code(), "malformed_subagent_marker");
        assert!(refusal.message().contains("CODEX_THREAD_ID"));
    }
    // An empty marker still means "no subagent".
    let env = [("CODEX_SESSION_ID", "c1"), ("CODEX_THREAD_ID", "")];
    let claim = run(&codex_table(), &env, None).unwrap();
    assert_eq!(claim.role, CallerRole::TopLevel);
}

#[test]
fn codex_managed_daemon_ancestor_is_refused() {
    let table = FakeTable(vec![
        rec(100, 90, 1000, &["herdr-threads"]),
        rec(90, 1, 900, &["codex", "app-server", "--managed-daemon"]),
    ]);
    let env = [("CODEX_SESSION_ID", "c1")];
    let refusal = run(&table, &env, None).unwrap_err();
    assert_eq!(refusal, EnvRefusal::ManagedDaemon { pid: 90 });
    assert_eq!(refusal.code(), "managed_daemon");
    assert!(refusal.message().contains("--no-daemon"));
}

#[test]
fn hermes_launcher_shapes_are_recognized() {
    let kind = |argv: &[&str]| classify(&rec(5, 1, 1, argv), None);
    assert_eq!(kind(&["/home/u/.hermes/bin/hermes", "-z"]), Some("hermes"));
    assert_eq!(
        kind(&["python3", "-I", "-c", "import hermes_cli.main as m"]),
        Some("hermes")
    );
    assert_eq!(kind(&["python3", "-m", "http.server"]), None);
    assert_eq!(kind(&["/usr/bin/hermes"]), Some("hermes"));
}

fn nested_table() -> FakeTable {
    FakeTable(vec![
        rec(100, 90, 1000, &["herdr-threads"]),
        rec(90, 70, 900, &["sh"]),
        rec(
            70,
            60,
            700,
            &["python3", "-I", "-c", "import hermes_cli.main"],
        ),
        rec(60, 50, 600, &["zsh"]),
        rec(50, 1, 500, &["claude"]),
    ])
}

#[test]
fn innermost_harness_wins_hermes_under_claude() {
    let env = [
        ("CLAUDE_CODE_SESSION_ID", "c1"),
        ("CLAUDE_PID", "50"),
        ("HERMES_SESSION_ID", "h1"),
    ];
    let claim = run(&nested_table(), &env, Some("a1")).unwrap();
    assert_eq!(claim.harness.as_str(), "hermes");
    assert_eq!(claim.native_session.as_str(), "h1");
    assert_eq!(claim.fingerprint.pid, 70);
    assert_eq!(claim.agent_id, None);
    assert_eq!(claim.role, CallerRole::TopLevel);
}

#[test]
fn missing_session_of_the_nearest_harness_is_refused_despite_outer_vars() {
    let env = [("CLAUDE_CODE_SESSION_ID", "c1"), ("CLAUDE_PID", "50")];
    assert_eq!(
        run(&nested_table(), &env, None).unwrap_err(),
        EnvRefusal::MissingSession {
            harness: "hermes",
            var: "HERMES_SESSION_ID"
        }
    );
}

#[test]
fn codex_without_session_variable_is_refused() {
    let env = [("CODEX_THREAD_ID", "t1")];
    assert_eq!(
        run(&codex_table(), &env, None).unwrap_err(),
        EnvRefusal::MissingSession {
            harness: "codex",
            var: "CODEX_SESSION_ID"
        }
    );
    // An empty value is as good as unset.
    let env = [("CODEX_SESSION_ID", "")];
    assert_eq!(
        run(&codex_table(), &env, None).unwrap_err().code(),
        "missing_session"
    );
}

#[test]
fn fingerprint_detects_pid_reuse() {
    let table = claude_table();
    let claim = run(&table, CLAUDE_ENV, None).unwrap();
    assert!(claim.fingerprint.is_alive(&table));
    let mut reused = claude_table();
    reused.0[2].start += 1;
    assert!(!claim.fingerprint.is_alive(&reused));
    assert!(!claim.fingerprint.is_alive(&FakeTable(vec![])));
}

#[test]
fn claude_pid_gone_falls_back_to_the_claude_shaped_ancestor() {
    let env = [("CLAUDE_CODE_SESSION_ID", "s1"), ("CLAUDE_PID", "4242")];
    let claim = run(&claude_table(), &env, None).unwrap();
    assert_eq!(claim.fingerprint.pid, 80);
    assert_eq!(claim.fingerprint.start, 800);
}

#[test]
fn claude_pid_record_wins_over_the_ancestor_when_both_exist() {
    // CLAUDE_PID names the launcher; the shaped ancestor is a child of it.
    let table = FakeTable(vec![
        rec(100, 90, 1000, &["herdr-threads"]),
        rec(90, 80, 900, &["claude"]),
        rec(80, 1, 800, &["bash", "wrapper"]),
    ]);
    let env = [("CLAUDE_CODE_SESSION_ID", "s1"), ("CLAUDE_PID", "80")];
    assert_eq!(run(&table, &env, None).unwrap().fingerprint.pid, 80);
}

#[test]
fn no_harness_ancestor_is_no_harness() {
    let table = FakeTable(vec![
        rec(100, 90, 1000, &["herdr-threads"]),
        rec(90, 80, 900, &["sh"]),
        rec(80, 1, 800, &["zsh"]),
    ]);
    let env = [("CLAUDE_CODE_SESSION_ID", "s1")];
    assert_eq!(run(&table, &env, None).unwrap_err(), EnvRefusal::NoHarness);
}

#[test]
fn ancestry_walk_is_bounded_and_cycle_safe() {
    let cycle = FakeTable(vec![
        rec(100, 90, 1, &["x"]),
        rec(90, 80, 1, &["sh"]),
        rec(80, 90, 1, &["sh"]),
    ]);
    assert_eq!(run(&cycle, &[], None).unwrap_err(), EnvRefusal::NoHarness);
    // A claude process 100 hops up is beyond the bound; one at hop 5 is not.
    let mut chain: Vec<ProcessRecord> = (0..100u32)
        .map(|i| rec(100 + i, 101 + i, 1, &["sh"]))
        .collect();
    chain.push(rec(200, 1, 1, &["claude"]));
    let env = [("CLAUDE_CODE_SESSION_ID", "s1")];
    assert_eq!(
        run(&FakeTable(chain), &env, None).unwrap_err(),
        EnvRefusal::NoHarness
    );
}

#[test]
fn agent_id_option_is_ignored_outside_claude() {
    let env = [("CODEX_SESSION_ID", "c1"), ("CODEX_THREAD_ID", "c1")];
    let codex = run(&codex_table(), &env, Some("a1")).unwrap();
    assert_eq!((codex.agent_id, codex.role), (None, CallerRole::TopLevel));
    let env = [("HERMES_SESSION_ID", "h1")];
    let hermes = run(&nested_table(), &env, Some("a1")).unwrap();
    assert_eq!((hermes.agent_id, hermes.role), (None, CallerRole::TopLevel));
}

#[test]
fn harness_fingerprint_finds_the_nearest_ancestor_of_that_harness() {
    let map = HashMap::from([("CLAUDE_PID".to_string(), "50".to_string())]);
    let lookup = move |name: &str| map.get(name).cloned();
    let table = nested_table();
    let inputs = EnvInputs {
        env: &lookup,
        self_pid: 100,
        agent_id: None,
        processes: &table,
    };
    assert_eq!(harness_fingerprint(&inputs, "claude").unwrap().pid, 50);
    assert_eq!(harness_fingerprint(&inputs, "hermes").unwrap().pid, 70);
    assert_eq!(
        harness_fingerprint(&inputs, "codex").unwrap_err(),
        EnvRefusal::NoHarness
    );
}

#[test]
fn unreadable_own_process_is_no_fingerprint() {
    let refusal = run(&FakeTable(vec![]), &[], None).unwrap_err();
    assert_eq!(refusal.code(), "no_fingerprint");
}

#[test]
fn refusal_codes_and_messages_are_stable() {
    let missing = EnvRefusal::MissingSession {
        harness: "codex",
        var: "CODEX_SESSION_ID",
    };
    assert_eq!(missing.code(), "missing_session");
    assert!(missing.message().contains("CODEX_SESSION_ID is not set"));
    assert!(
        missing
            .message()
            .starts_with("codex caller has no session id")
    );
    let none = EnvRefusal::NoFingerprint {
        harness: "claude",
        detail: "gone".into(),
    };
    assert_eq!(none.code(), "no_fingerprint");
    assert_eq!(
        none.message(),
        "claude caller has no liveness fingerprint: gone"
    );
    let daemon = EnvRefusal::ManagedDaemon { pid: 7 };
    assert_eq!(daemon.code(), "managed_daemon");
    assert!(daemon.message().contains("(pid 7)"));
    assert_eq!(EnvRefusal::NoHarness.code(), "no_harness");
    assert!(
        EnvRefusal::NoHarness
            .message()
            .contains("claude, codex, hermes or other registered agent harness")
    );
}

/// A throwaway `HOME` holding agy's witness tree, removed on drop.
struct AgyHome(std::path::PathBuf);
impl AgyHome {
    fn new() -> Self {
        let home = std::env::temp_dir().join(format!("ht-agy-derive-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(home.join(".gemini/antigravity-cli")).unwrap();
        Self(home)
    }
    fn data(&self) -> std::path::PathBuf {
        self.0.join(".gemini/antigravity-cli")
    }
    fn presence_lock(&self, conversation: &str) {
        let dir = self.data().join("presence");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{conversation}.lock")), b"").unwrap();
    }
    fn subagent_listing(&self, parent: &str, child: &str) {
        let dir = self
            .data()
            .join("brain")
            .join(parent)
            .join(".system_generated/subagents");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{child}.json")), b"{}").unwrap();
    }
    fn home(&self) -> &str {
        self.0.to_str().unwrap()
    }
}
impl Drop for AgyHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn agy_table() -> FakeTable {
    FakeTable(vec![
        rec(100, 90, 1000, &["herdr-threads", "inbox"]),
        rec(90, 80, 900, &["sh"]),
        rec(80, 1, 800, &["agy"]),
    ])
}

fn run_agy(home: &AgyHome, conversation: &str) -> Result<EnvClaim, EnvRefusal> {
    run(
        &agy_table(),
        &[
            ("HOME", home.home()),
            ("ANTIGRAVITY_CONVERSATION_ID", conversation),
        ],
        None,
    )
}

fn assert_agy_unattributed(result: Result<EnvClaim, EnvRefusal>) {
    match result {
        Err(refusal @ EnvRefusal::UnattributedSession { harness: "agy", .. }) => {
            assert_eq!(refusal.code(), "unattributed_session");
        }
        other => panic!("expected an agy unattributed_session refusal, got {other:?}"),
    }
}

#[test]
fn agy_child_conversation_is_a_subagent_claim_of_its_parent() {
    let home = AgyHome::new();
    home.subagent_listing("parent-1", "child-1");
    let claim = run_agy(&home, "child-1").unwrap();
    assert_eq!(claim.native_session.as_str(), "parent-1");
    assert_eq!(claim.agent_id, Some(AgentId::new("child-1")));
    assert_eq!(claim.role, CallerRole::Subagent);
    assert_eq!(claim.provenance, "cooperative_env_session");
    assert_eq!(
        claim.fingerprint,
        LivenessFingerprint {
            pid: 80,
            start: 800
        }
    );
}

#[test]
fn agy_top_level_conversation_with_a_presence_lock() {
    let home = AgyHome::new();
    home.presence_lock("conv-1");
    let claim = run_agy(&home, "conv-1").unwrap();
    assert_eq!(claim.native_session.as_str(), "conv-1");
    assert_eq!(claim.agent_id, None);
    assert_eq!(claim.role, CallerRole::TopLevel);
    assert_eq!(claim.fingerprint.pid, 80);
}

#[test]
fn agy_conversation_without_lock_or_listing_is_unattributed() {
    let home = AgyHome::new();
    assert_agy_unattributed(run_agy(&home, "conv-1"));
}

#[test]
fn agy_unreadable_listing_is_unattributed_never_top_level() {
    let home = AgyHome::new();
    // `brain` is a regular file: its `read_dir` fails with not-a-directory.
    std::fs::write(home.data().join("brain"), b"").unwrap();
    assert_agy_unattributed(run_agy(&home, "conv-1"));
}

#[test]
fn agy_invalid_conversation_id_is_unattributed() {
    let home = AgyHome::new();
    // Printable (so it parses as a session id) but path material: `../x` would escape `presence/`.
    home.presence_lock("bad!id");
    assert_agy_unattributed(run_agy(&home, "bad!id"));
}
