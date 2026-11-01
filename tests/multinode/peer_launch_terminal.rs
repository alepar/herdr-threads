//! Peer launch onto herdr-core (`LaunchTarget::NewTerminal`, cap
//! `peer.launch.terminal.v1`) and the `peer-tab-create` alias of
//! `internal peer-target-create`.

use crate::support::{self, P1, P2};
use herdr_threads::sync::frame::PEER_LAUNCH_TERMINAL_CAP;
use herdr_threads::test_support::fake_hc::FakeHc;
use herdr_threads::test_support::multinode::{Cluster, ClusterOptions};

/// `node`'s tag as `observer` knows it.
fn tag_of(cluster: &Cluster, observer: usize, node: usize) -> String {
    let uuid = cluster.node_uuid(node).to_string();
    cluster
        .scalar(
            observer,
            "SELECT tag FROM node_directory WHERE node_uuid=?1",
            &[&uuid],
        )
        .expect("a known node")
}

/// A handoff into a new terminal on `machine`'s node (the test-support switch turns the
/// `--new-tab` target into a `NewTerminal`), as the seat on node 0.
fn handoff_terminal(
    cluster: &Cluster,
    seat: &herdr_threads::test_support::multinode::Seat,
    machine: &str,
) -> (i32, String, String) {
    cluster.text_as_env(
        Some(seat),
        0,
        &[
            "handoff",
            "--new-tab",
            "review",
            "--space",
            &format!("w1@{machine}"),
            "--cwd",
            "/tmp",
            "--kind",
            "claude",
            "--",
            "look at this",
        ],
        &[
            ("HT_TEST_REMOTE_TERMINAL", "1"),
            ("HT_TEST_REMOTE_LAUNCH_TIMEOUT_MS", "6000"),
        ],
    )
}

// Kills: a NewTerminal sent to a peer that never advertised the capability (an older
// peer reads it as `unknown`), a capable peer's terminal not created on its herdr-core,
// and a terminal that inherits a parent, notifies the coordinator or is kept.
#[test]
fn incapable_peer_never_sent_new_terminal() {
    let mut cluster = Cluster::start(
        "mn-peer-term-nocap",
        crate::BIN,
        2,
        ClusterOptions {
            links: vec![(0, 1)],
            env: vec![(
                1,
                "HT_TEST_WITHHOLD_CAPS".to_owned(),
                PEER_LAUNCH_TERMINAL_CAP.to_owned(),
            )],
            ..Default::default()
        },
    );
    cluster.add_host(0, &[P1]);
    cluster.add_host(1, &[P1, P2]);
    let a = cluster.seat(0, P1);
    let _b = cluster.seat(1, P1);
    support::wait_seats(&cluster, 0, 2);
    cluster.wait_converged(crate::WAIT);
    let b_tag = tag_of(&cluster, 0, 1);

    let (code, stdout, stderr) = handoff_terminal(&cluster, &a, &b_tag);
    assert_ne!(code, 0, "{stdout}");
    assert!(
        stderr.contains(&format!("needs {PEER_LAUNCH_TERMINAL_CAP}")),
        "{stderr}"
    );
    // Nothing reached node 1's launcher: no request row.
    assert_eq!(
        cluster
            .scalar(1, "SELECT count(*) FROM peer_requests", &[])
            .as_deref(),
        Some("0")
    );
}

#[test]
fn capable_peer_new_terminal_creates_on_fake_hc() {
    let hc = FakeHc::start();
    let mut cluster = Cluster::start(
        "mn-peer-term-cap",
        crate::BIN,
        2,
        ClusterOptions {
            links: vec![(0, 1)],
            ..Default::default()
        },
    );
    cluster.add_host(0, &[P1]);
    let a = cluster.seat(0, P1);
    support::wait_seats(&cluster, 0, 1);
    // Node 1 has only a herdr-core, its startup host.
    cluster.restart_with_host(1, hc.socket());
    cluster.wait_converged(crate::WAIT);
    let b_tag = tag_of(&cluster, 0, 1);

    // The CLI gives up while the gate shim (a later task) reports no seat; the terminal
    // exists by then.
    // (Node 1 was restarted: the first tries may find its link still down.)
    let (code, stdout, stderr) = cluster.wait_until("the link to node 1", crate::WAIT, || {
        let out = handoff_terminal(&cluster, &a, &b_tag);
        (!out.2.contains("unreachable")).then_some(out)
    });
    assert_ne!(code, 0, "{stdout}{stderr}");
    let runs = hc.calls_to("terminal.run");
    assert_eq!(runs.len(), 1, "{runs:?} {stdout}{stderr}");
    let params = &runs[0].params;
    assert_eq!(params["keep"], false, "{params}");
    assert_eq!(params["no_notify"], true, "{params}");
    assert_eq!(params["no_parent"], true, "{params}");
    assert!(params.get("parent").is_none(), "{params}");
    assert!(params.get("caller").is_none(), "{params}");
    let argv = params["argv"].as_array().unwrap();
    assert!(
        argv.iter().any(|a| a == "launch-gate") && argv.iter().any(|a| a == "--kind=claude"),
        "{params}"
    );
    let a_tag = tag_of(&cluster, 1, 0);
    assert_eq!(
        params["labels"]["ht.label"],
        format!("review [from {a_tag}]"),
        "{params}"
    );
    assert!(params["labels"].get("herdr.space").is_none(), "{params}");
    assert_eq!(
        cluster
            .scalar(1, "SELECT pane FROM peer_requests", &[])
            .as_deref(),
        Some("t1")
    );
}

// Kills: an alias that parses but acts differently (other output shape or other
// request) from the new name.
#[test]
fn peer_tab_create_alias_matches_peer_target_create() {
    let hc = FakeHc::start();
    let run = |name: &str| {
        let output = std::process::Command::new(crate::BIN)
            .env_clear()
            .args(["--json", "--host-endpoint"])
            .arg(hc.socket())
            .args([
                "internal",
                name,
                "--workspace=sp1",
                "--label=x",
                "--cwd=/tmp",
            ])
            .args(["--", "/bin/true"])
            .output()
            .unwrap();
        (
            output.status.code(),
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        )
    };
    let (new_code, new_out) = run("peer-target-create");
    let (old_code, old_out) = run("peer-tab-create");
    assert_eq!((new_code, old_code), (Some(0), Some(0)));
    let keys = |v: &serde_json::Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert_eq!(keys(&new_out), keys(&old_out));
    assert!(new_out["terminal"].is_string(), "{new_out}");
    let runs = hc.calls_to("terminal.run");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].params["labels"]["herdr.space"], "sp1");
    let strip = |mut p: serde_json::Value| {
        p["labels"]["herdr.tab"] = "".into();
        p["labels"]["ht.bootstrap"] = "".into();
        p
    };
    assert_eq!(strip(runs[0].params.clone()), strip(runs[1].params.clone()));
}
