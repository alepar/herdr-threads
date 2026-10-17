//! Send-time attribution must survive replication without remote bindings.
use crate::support::{self, P1, P2};

#[test]
fn user_intent_and_frozen_author_role_survive_two_node_replication() {
    let cluster = support::cluster("mn-user-intent", 2, &[(0, 1)], &[0, 1]);
    let a = cluster.seat(0, P1);
    let b = cluster.seat(1, P1);
    let human = cluster.human_seat(0, P2);
    for node in [0, 1] {
        support::wait_seats(&cluster, node, 3);
    }
    for intent in ["rule", "query", "request"] {
        let (thread, message) = support::send(
            &cluster,
            &a,
            &[
                "--seat",
                &b.id,
                "--body",
                intent,
                "--relays-user",
                "--user-intent",
                intent,
            ],
        );
        cluster.wait_converged(crate::WAIT);
        assert_attribution(&cluster, &b, &thread, &message, "agent", true, intent);
    }
    for relay in [false, true] {
        let mut command = cluster.human_pane_command(&human);
        command.args([
            "--json",
            "send",
            "--seat",
            &b.id,
            "--body",
            "human rule",
            "--user-intent",
            "rule",
        ]);
        if relay {
            command.arg("--relays-user");
        }
        let output = command.output().unwrap();
        assert!(output.status.success(), "{output:?}");
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let result = &value["result"];
        cluster.wait_converged(crate::WAIT);
        assert_attribution(
            &cluster,
            &b,
            result["thread"].as_str().unwrap(),
            result["data"].as_str().unwrap(),
            "human",
            relay,
            "rule",
        );
    }
}

fn assert_attribution(
    cluster: &herdr_threads::test_support::multinode::Cluster,
    reader: &herdr_threads::test_support::multinode::Seat,
    thread: &str,
    message: &str,
    role: &str,
    relay: bool,
    intent: &str,
) {
    for node in [0, 1] {
        assert_eq!(cluster.scalar(node, "SELECT author_role||'/'||relays_user||'/'||user_intent||'/'||author_role_backfilled FROM messages WHERE id=?1", &[&message]), Some(format!("{role}/{}/{intent}/0", i32::from(relay))), "node {node}: message attribution (a failed apply leaves no row)");
    }
    let (code, text, stderr) = cluster.text_as(Some(reader), reader.node, &["read", thread]);
    assert_eq!(code, 0, "{stderr}");
    assert!(text.contains(&format!("[{intent}]")), "{text}");
    assert_eq!(text.contains("[human]"), role == "human", "{text}");
    assert_eq!(text.contains("[relays user]"), relay, "{text}");
}
