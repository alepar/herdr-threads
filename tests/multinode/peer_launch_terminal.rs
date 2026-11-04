//! Peer launch onto herdr-core (`LaunchTarget::NewTerminal`, cap
//! `peer.launch.terminal.v1`) and the `peer-tab-create` alias of
//! `internal peer-target-create`.

use crate::support::{self, P1, P2};
use herdr_threads::sync::frame::PEER_HANDOFF_CAP;
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

/// A handoff into a new terminal (`terminal`: `--new-terminal --space @MACHINE`) or a new tab
/// (`--new-tab review --space w1@MACHINE`) on `machine`'s node, as the seat on node 0.
fn handoff_into(
    cluster: &Cluster,
    seat: &herdr_threads::test_support::multinode::Seat,
    machine: &str,
    terminal: bool,
    timeout_ms: &str,
) -> (i32, String, String) {
    let env = vec![("HT_TEST_REMOTE_LAUNCH_TIMEOUT_MS", timeout_ms)];
    let target: Vec<String> = if terminal {
        vec![
            "--new-terminal".into(),
            "--space".into(),
            format!("@{machine}"),
        ]
    } else {
        vec![
            "--new-tab".into(),
            "review".into(),
            "--space".into(),
            format!("w1@{machine}"),
        ]
    };
    let mut args = vec!["handoff"];
    args.extend(target.iter().map(String::as_str));
    args.extend([
        "--cwd",
        "/tmp",
        "--kind",
        "claude",
        "--summary",
        "review the change",
        "--",
        "look at this",
    ]);
    cluster.text_as_env(Some(seat), 0, &args, &env)
}

fn handoff_terminal(
    cluster: &Cluster,
    seat: &herdr_threads::test_support::multinode::Seat,
    machine: &str,
) -> (i32, String, String) {
    handoff_into(cluster, seat, machine, true, "6000")
}

/// A scratch directory with a stand-in `claude` that records `<pid> <terminal>` in the file
/// `$HT_STANDIN_MARKER` and then sleeps; and the PATH that finds it first.
fn standin() -> (std::path::PathBuf, String) {
    use std::os::unix::fs::PermissionsExt;
    let dir = herdr_threads::test_support::scratch::temp_path(format!(
        "ht-standin-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("claude");
    std::fs::write(
        &exe,
        "#!/bin/sh\necho \"$$ $HC_TERMINAL_ID\" > \"$HT_STANDIN_MARKER\"\nexec sleep 300\n",
    )
    .unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    (dir, path)
}

/// Two nodes: node 0 with a Herdr host and a seat, node 1 with only the fake herdr-core
/// (really running its terminals' programs). Returns the cluster, the requesting seat, node 1's
/// tag and the marker file the stand-in harness writes.
fn core_cluster(
    label: &str,
    hc: &FakeHc,
) -> (
    Cluster,
    herdr_threads::test_support::multinode::Seat,
    String,
    std::path::PathBuf,
) {
    let (dir, path) = standin();
    let marker = dir.join("marker");
    hc.exec_runs(&[("HT_STANDIN_MARKER", marker.to_str().unwrap())]);
    let mut cluster = Cluster::start(
        label,
        crate::BIN,
        2,
        ClusterOptions {
            links: vec![(0, 1)],
            env: vec![(1, "PATH".to_owned(), path)],
            ..Default::default()
        },
    );
    cluster.add_host(0, &[P1]);
    // A handoff needs a daemon started with its host (guarded bootstrap).
    cluster.stop_node(0);
    cluster.start_node(0);
    let a = cluster.seat(0, P1);
    support::wait_seats(&cluster, 0, 1);
    // Node 1 has only a herdr-core, its startup host.
    cluster.restart_with_host(1, hc.socket());
    cluster.wait_converged(crate::WAIT);
    let tag = tag_of(&cluster, 0, 1);
    (cluster, a, tag, marker)
}

/// What a launch through the gate leaves on node 1 and the fake core: the request's row is
/// `launched` on `terminal` with a seat and a gate-only record, that seat has one
/// `managed_launch` binding, the stand-in harness ran as the terminal's own process, and the
/// task reached the seat.
fn assert_gate_launch(
    cluster: &Cluster,
    hc: &FakeHc,
    marker: &std::path::Path,
    terminal: &str,
    stdout: &str,
) {
    let text = cluster.wait_until("the stand-in harness to run", crate::WAIT, || {
        std::fs::read_to_string(marker)
            .ok()
            .filter(|t| t.ends_with('\n'))
    });
    let (pid, seen) = text.trim().split_once(' ').expect("`<pid> <terminal>`");
    assert_eq!(seen, terminal, "{text}");
    let record = hc.terminal(terminal).expect("the created terminal");
    assert_eq!(
        record.pid,
        Some(pid.parse().unwrap()),
        "exec kept the shim's pid"
    );
    assert_eq!(
        cluster
            .scalar(1, "SELECT state FROM peer_requests", &[])
            .as_deref(),
        Some("launched"),
        "{stdout}"
    );
    assert_eq!(
        cluster
            .scalar(1, "SELECT pane FROM peer_requests", &[])
            .as_deref(),
        Some(terminal)
    );
    assert_eq!(
        cluster
            .scalar(1, "SELECT gate_attempt FROM peer_requests", &[])
            .as_deref(),
        Some("1")
    );
    let created: herdr_threads::ports::CreatedTarget = serde_json::from_str(
        &cluster
            .scalar(1, "SELECT gate_created FROM peer_requests", &[])
            .expect("a recorded creation"),
    )
    .unwrap();
    assert_eq!(created.terminal.as_str(), terminal);
    created.validate().unwrap();
    let seat = cluster
        .scalar(1, "SELECT seat FROM peer_requests", &[])
        .filter(|s| s != "NULL")
        .expect("the launch's seat");
    assert_eq!(
        cluster
            .scalar(
                1,
                "SELECT count(*) FROM occupant_bindings WHERE seat_id=?1",
                &[&seat],
            )
            .as_deref(),
        Some("1")
    );
    assert_eq!(
        cluster
            .scalar(
                1,
                "SELECT observation_provenance FROM occupant_bindings WHERE seat_id=?1",
                &[&seat],
            )
            .as_deref(),
        Some("managed_launch")
    );
    // The task is authored after `Launched{seat}`, by normal delivery, and
    // replicates to node 1: wait for it, then require exactly one.
    let count = cluster.wait_until("the task to reach node 1", crate::WAIT, || {
        cluster
            .scalar(
                1,
                "SELECT count(*) FROM messages WHERE body LIKE '%look at this%'",
                &[],
            )
            .filter(|count| count != "0")
    });
    assert_eq!(count, "1", "{stdout}");
}

/// The `terminal.run` params every gate launch sends.
fn assert_run_params(params: &serde_json::Value) {
    assert_eq!(params["keep"], false, "{params}");
    assert_eq!(params["no_notify"], true, "{params}");
    assert_eq!(params["no_parent"], true, "{params}");
    assert!(params.get("parent").is_none(), "{params}");
    assert!(params.get("caller").is_none(), "{params}");
    let argv = params["argv"].as_array().unwrap();
    let has = |needle: &str| argv.iter().any(|a| a == needle);
    assert!(
        has("launch-gate") && has("--state-dir") && has("--bootstrap"),
        "{params}"
    );
    assert!(!argv.iter().any(|a| a == "--kind=claude"), "{params}");
    // The recovery hints: the label names the sender, the bootstrap hint is the call id.
    let label = params["labels"]["ht.label"].as_str().unwrap_or_default();
    assert!(label.ends_with(']') && label.contains("[from "), "{params}");
    let hint = params["labels"]["ht.bootstrap"]
        .as_str()
        .unwrap_or_default();
    assert!(!hint.trim().is_empty(), "{params}");
}

// Kills: a handoff into a new terminal sent to a peer that never advertised the handoff
// capability (an older peer reads the target as `unknown` and drops the message), a capable peer's terminal not created on its herdr-core,
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
                PEER_HANDOFF_CAP.to_owned(),
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
        stderr.contains(&format!("needs {PEER_HANDOFF_CAP}")),
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

// Kills: a peer launch onto a herdr-core that records only the argv (the shim never ran the
// harness and no seat was reported), a gate argv the real shim cannot parse, and a terminal that
// inherits a parent, notifies the coordinator or is kept.
#[test]
fn capable_peer_new_terminal_creates_on_fake_hc() {
    let hc = FakeHc::start();
    let (cluster, a, b_tag, marker) = core_cluster("mn-peer-term-cap", &hc);
    // (Node 1 was restarted: the first tries may find its link still down.)
    let (code, stdout, stderr) = cluster.wait_until("the link to node 1", crate::WAIT, || {
        let out = handoff_into(&cluster, &a, &b_tag, true, "30000");
        (!out.2.contains("unreachable")).then_some(out)
    });
    assert_eq!(code, 0, "{stdout}{stderr}");
    let runs = hc.calls_to("terminal.run");
    assert_eq!(runs.len(), 1, "{runs:?} {stdout}{stderr}");
    let params = &runs[0].params;
    assert_run_params(params);
    // A new terminal carries no app labels (herdr.*), only the two ht.* recovery hints (design,
    // CLI and placement; the unit test `new_terminal_labels_and_parent_caller` pins the host side).
    assert!(
        params["labels"].as_object().is_some_and(|l| {
            let mut keys: Vec<&str> = l.keys().map(String::as_str).collect();
            keys.sort_unstable();
            keys == ["ht.bootstrap", "ht.label"]
        }),
        "{params}"
    );
    assert_gate_launch(&cluster, &hc, &marker, "t1", &stdout);
}

// Kills: the production path (a new tab in an app space of the core) missing the gate-only
// record or the seat that a new terminal gets.
#[test]
fn peer_new_tab_on_hc_runs_through_the_gate() {
    use herdr_threads::test_support::fake_hc::FakeTerminal;
    let hc = FakeHc::start();
    let mut anchor = FakeTerminal::live("t1");
    anchor.labels = [("herdr.space", "w1"), ("herdr.tab", "x")]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    hc.add_terminal(anchor);
    let (cluster, a, b_tag, marker) = core_cluster("mn-peer-tab-cap", &hc);
    let (code, stdout, stderr) = cluster.wait_until("the link to node 1", crate::WAIT, || {
        let out = handoff_into(&cluster, &a, &b_tag, false, "30000");
        (!out.2.contains("unreachable")).then_some(out)
    });
    assert_eq!(code, 0, "{stdout}{stderr}");
    let runs = hc.calls_to("terminal.run");
    assert_eq!(runs.len(), 1, "{runs:?} {stdout}{stderr}");
    let params = &runs[0].params;
    assert_run_params(params);
    assert_eq!(params["labels"]["herdr.space"], "w1", "{params}");
    assert_gate_launch(&cluster, &hc, &marker, "t2", &stdout);
}

/// Runs the real shim as a herdr-core terminal `terminal` of `hc` would, for `bootstrap`; its
/// exit code and stderr.
fn run_shim(
    cluster: &Cluster,
    hc: &FakeHc,
    terminal: &str,
    bootstrap: &str,
    marker: &std::path::Path,
) -> (Option<i32>, String) {
    use herdr_threads::test_support::spawn::{SpawnOwned, command};
    let (dir, path) = standin();
    let _ = dir;
    let child = command(crate::BIN)
        .env("PATH", &path)
        .env("HC_TERMINAL_ID", terminal)
        .env("HC_TERMINAL_SOCKET", hc.socket())
        .env("HT_STANDIN_MARKER", marker)
        .arg("--state-dir")
        .arg(cluster.state_dir(1))
        .args(["internal", "launch-gate", "--bootstrap", bootstrap, "--"])
        .arg("claude")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn_owned()
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

// Kills: a gate that releases a harness for a request it has no record of (a stranger's
// bootstrap, a forged id): the shim must be refused and nothing may start.
#[test]
fn gate_without_a_record_refuses_and_starts_nothing() {
    let hc = FakeHc::start();
    let (cluster, _a, _tag, _marker) = core_cluster("mn-peer-gate-none", &hc);
    let marker = herdr_threads::test_support::scratch::temp_path(format!(
        "ht-nomark-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let unknown = format!("{}:1", uuid::Uuid::new_v4());
    let (code, stderr) = run_shim(&cluster, &hc, "t1", &unknown, &marker);
    assert_ne!(code, Some(0), "{stderr}");
    assert!(
        stderr.contains("launch gate refused: unknown bootstrap"),
        "{stderr}"
    );
    assert!(!marker.exists(), "the harness ran");
}

// Kills: a gate-only record that releases any terminal of the core, not the one it created.
#[test]
fn gate_of_a_foreign_terminal_refuses() {
    let hc = FakeHc::start();
    let (cluster, a, b_tag, marker) = core_cluster("mn-peer-gate-foreign", &hc);
    let (code, stdout, stderr) = cluster.wait_until("the link to node 1", crate::WAIT, || {
        let out = handoff_into(&cluster, &a, &b_tag, true, "30000");
        (!out.2.contains("unreachable")).then_some(out)
    });
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert_gate_launch(&cluster, &hc, &marker, "t1", &stdout);
    let id = cluster
        .scalar(1, "SELECT id FROM peer_requests", &[])
        .unwrap();
    let second = marker.with_file_name("second-marker");
    let (code, stderr) = run_shim(&cluster, &hc, "t99", &format!("{id}:1"), &second);
    assert_ne!(code, Some(0), "{stderr}");
    assert!(
        stderr.contains("terminal is not the bootstrap's recipient"),
        "{stderr}"
    );
    assert!(!second.exists(), "a second harness ran");
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

/// A handoff into a new terminal on `machine` with `--profile-dir dir`.
fn handoff_terminal_with_profile(
    cluster: &Cluster,
    seat: &herdr_threads::test_support::multinode::Seat,
    machine: &str,
    dir: &std::path::Path,
) -> (i32, String, String) {
    let target = format!("@{machine}");
    cluster.text_as_env(
        Some(seat),
        0,
        &[
            "handoff",
            "--new-terminal",
            "--space",
            &target,
            "--cwd",
            "/tmp",
            "--kind",
            "claude",
            "--summary",
            "review the change",
            "--profile-dir",
            dir.to_str().unwrap(),
            "--",
            "look at this",
        ],
        &[("HT_TEST_REMOTE_LAUNCH_TIMEOUT_MS", "30000")],
    )
}

fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir = herdr_threads::test_support::scratch::temp_path(format!(
        "ht-{name}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

// Kills: a gate argv without the frozen prompt or the profile root, a harness released before
// the staged Send exists (the gate view must stay closed until the task message is on the
// target), and a gate that does not open once it is.
#[test]
fn handoff_new_terminal_gate_waits_for_staging() {
    use herdr_threads::test_support::spawn::{SpawnOwned, command};
    let hc = FakeHc::start();
    let (cluster, a, b_tag, marker) = core_cluster("mn-peer-term-handoff", &hc);
    let profile = scratch_dir("profile");
    let (code, stdout, stderr) = cluster.wait_until("the link to node 1", crate::WAIT, || {
        let out = handoff_terminal_with_profile(&cluster, &a, &b_tag, &profile);
        (!out.2.contains("unreachable")).then_some(out)
    });
    assert_eq!(code, 0, "{stdout}{stderr}");
    let runs = hc.calls_to("terminal.run");
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_run_params(&runs[0].params);
    let argv: Vec<String> = runs[0].params["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    let at = argv.iter().position(|x| x == "--config-root").unwrap();
    assert_eq!(
        argv[at + 1],
        format!("claude={}", profile.display()),
        "{argv:?}"
    );
    let tail = argv[argv.iter().rposition(|x| x == "--").unwrap()..].join(" ");
    assert!(
        tail.contains("review the change") && tail.contains(&format!("from: {}", a.id)),
        "the harness argv carries the prompt: {tail}"
    );
    assert_gate_launch(&cluster, &hc, &marker, "t1", &stdout);

    // Close the gate: un-stage the message. A second shim for the same bootstrap must wait,
    // not exec the harness; once the message is back it proceeds.
    let id = cluster
        .scalar(1, "SELECT id FROM peer_requests", &[])
        .unwrap();
    let staged = cluster
        .scalar(1, "SELECT handoff_message FROM peer_requests", &[])
        .expect("a staged message");
    cluster.write_sql(1, "UPDATE peer_requests SET handoff_message=NULL");
    let (dir, path) = standin();
    let _ = dir;
    let second = marker.with_file_name("second-marker");
    let mut child = command(crate::BIN)
        .env("PATH", &path)
        .env("HC_TERMINAL_ID", "t1")
        .env("HC_TERMINAL_SOCKET", hc.socket())
        .env("HT_STANDIN_MARKER", &second)
        .arg("--state-dir")
        .arg(cluster.state_dir(1))
        .args([
            "internal",
            "launch-gate",
            "--bootstrap",
            &format!("{id}:1"),
            "--",
        ])
        .arg("claude")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn_owned()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_secs(3));
    assert!(
        child.try_wait().unwrap().is_none() && !second.exists(),
        "the gate released a harness without the staged Send"
    );
    cluster.write_sql(
        1,
        &format!("UPDATE peer_requests SET handoff_message='{staged}'"),
    );
    cluster.wait_until("the gate to open on the staged Send", crate::WAIT, || {
        second.exists().then_some(())
    });
    let _ = child.kill();
    let _ = child.wait();
}

// Kills: a handoff refused by the target (a profile directory that is not there) that still
// creates a terminal or authors a fact, and a refusal that leaves the sender without a ledger
// answer.
#[test]
fn handoff_new_terminal_refusal_creates_no_terminal() {
    let hc = FakeHc::start();
    let (cluster, a, b_tag, _marker) = core_cluster("mn-peer-term-refuse", &hc);
    cluster.wait_until("the link to node 1", crate::WAIT, || {
        let out = cluster.text_as_env(Some(&a), 0, &["peer", "list"], &[]);
        out.1.contains("linked").then_some(())
    });
    let before = (cluster.kinds(0), cluster.kinds(1));
    let missing = scratch_dir("missing").join("not-there");
    let (code, _, stderr) = handoff_terminal_with_profile(&cluster, &a, &b_tag, &missing);
    assert_ne!(code, 0);
    assert!(stderr.contains("refused"), "{stderr}");
    assert!(
        hc.calls_to("terminal.run").is_empty(),
        "a terminal was created"
    );
    assert_eq!((cluster.kinds(0), cluster.kinds(1)), before);
    assert_eq!(
        cluster
            .scalar(0, "SELECT state FROM handoff_obligations", &[])
            .as_deref(),
        Some("failed")
    );
}

// Kills: a bare `launch --new-terminal` turned into a handoff (a prompt or config root in the
// gate argv, a ledger row, a message).
#[test]
fn bare_new_terminal_unchanged() {
    let hc = FakeHc::start();
    let (cluster, a, b_tag, _marker) = core_cluster("mn-peer-term-bare", &hc);
    let target = format!("@{b_tag}");
    let (code, stdout, stderr) = cluster.wait_until("the link to node 1", crate::WAIT, || {
        let out = cluster.text_as_env(
            Some(&a),
            0,
            &[
                "launch",
                "--new-terminal",
                "--space",
                &target,
                "--cwd",
                "/tmp",
                "--kind",
                "claude",
            ],
            &[("HT_TEST_REMOTE_LAUNCH_TIMEOUT_MS", "30000")],
        );
        (!out.2.contains("unreachable")).then_some(out)
    });
    assert_eq!(code, 0, "{stdout}{stderr}");
    let runs = hc.calls_to("terminal.run");
    assert_eq!(runs.len(), 1, "{runs:?}");
    let argv = runs[0].params["argv"].to_string();
    assert!(
        !argv.contains("--config-root") && !argv.contains("from:"),
        "{argv}"
    );
    assert_eq!(
        cluster
            .scalar(0, "SELECT count(*) FROM handoff_obligations", &[])
            .as_deref(),
        Some("0")
    );
    for node in 0..2 {
        assert!(
            !cluster.kinds(node).contains_key("message.sent"),
            "no message authored (node {node})"
        );
    }
    assert_eq!(
        cluster
            .scalar(
                1,
                "SELECT count(*) FROM messages WHERE kind='ordinary'",
                &[]
            )
            .as_deref(),
        Some("0")
    );
}
