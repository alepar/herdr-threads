//! Remote addresses (`SEAT@MACHINE`): a seat argument with a machine suffix
//! is resolved by that machine's own daemon against its live topology and
//! seat table; the asking node only learns the seat ID. A machine name that
//! matches several nodes is refused listing them; a peer without
//! `addr.resolve.v1` and an unreachable peer are refused, never resolved
//! locally.

use crate::support::{self, P1, P2};
use herdr_threads::test_support::multinode::{Cluster, ClusterOptions};

const P3: &str = "w1:p3";

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

/// The seat `seat inspect ADDRESS` names, as `as_seat` on node 0.
fn inspect(
    cluster: &Cluster,
    as_seat: &herdr_threads::test_support::multinode::Seat,
    address: &str,
) -> Result<String, String> {
    let (code, value, stderr) = cluster.run_as(Some(as_seat), 0, &["seat", "inspect", address]);
    if code == 0 {
        Ok(value["result"]["data"]["summary"]["seat"]
            .as_str()
            .unwrap_or_default()
            .to_owned())
    } else {
        Err(format!("{stderr}{value}"))
    }
}

// Kills: a suffix resolved against the local topology, a remote seat not
// found by its path, the machine matched by only one of uuid/tag/name, a
// shared host name accepted, an unseated remote pane allocated or silently
// mapped, a capless peer answered by a local fallback, and an unreachable
// peer not reported.
#[test]
fn remote_addresses_resolve_on_their_home_node() {
    let mut env = Vec::new();
    for (node, label) in [(0, "alpha"), (1, "beta"), (2, "beta.lab")] {
        env.push((node, "HT_TEST_HOST_LABEL".to_owned(), label.to_owned()));
    }
    env.push((
        2,
        "HT_TEST_WITHHOLD_CAPS".to_owned(),
        "addr.resolve.v1".to_owned(),
    ));
    let cluster = {
        let mut cluster = Cluster::start(
            "mn-remote-addr",
            crate::BIN,
            3,
            ClusterOptions {
                links: vec![(0, 1), (0, 2)],
                env,
                ..Default::default()
            },
        );
        // Local labels that carry an `@`: one per collision case.
        let b_prefix = cluster.node_uuid(1).to_string()[..8].to_owned();
        let collide_remote = format!("w1:p6=w1:p1@{b_prefix}");
        let narrowed_local = format!("w1:p8=ci@{}", &cluster.node_uuid(1).to_string()[..9]);
        cluster.add_host(
            0,
            &[
                P1,
                "w1:p4=w1:p1@beta.lab",
                "w1:p5=user@nowhere",
                &collide_remote,
                "w1:p7=w1:p1@alpha",
                &narrowed_local,
            ],
        );
        cluster.add_host(1, &[P1, P2]);
        cluster.add_host(2, &[P1, P3]);
        cluster
    };
    let a = cluster.seat(0, P1);
    let b = cluster.seat(1, P1);
    let c = cluster.seat(2, P1);
    support::wait_seats(&cluster, 0, 3);
    support::wait_seats(&cluster, 1, 3);
    cluster.wait_converged(crate::WAIT);
    let b_tag = tag_of(&cluster, 0, 1);
    let b_uuid = cluster.node_uuid(1).to_string();

    // A path on beta by tag, uuid prefix and full uuid: beta's own seat.
    // A 9-hex prefix: the 8-hex one is a colliding local label below.
    for machine in [b_tag.as_str(), &b_uuid[..9], &b_uuid] {
        let address = format!("w1/w1:t1/{P1}@{machine}");
        assert_eq!(
            inspect(&cluster, &a, &address),
            Ok(b.id.clone()),
            "{address}"
        );
    }
    // `beta` names both beta and beta.lab (its first DNS label): refused,
    // listing both uuids.
    let err = inspect(&cluster, &a, &format!("{P1}@beta")).unwrap_err();
    assert!(err.contains("ambiguous machine beta: 2 matches"), "{err}");
    assert!(err.contains(&b_uuid), "{err}");
    assert!(err.contains("pass @<uuid>"), "{err}");
    // A send by remote path reaches beta's seat.
    let (_, message) = support::send(
        &cluster,
        &a,
        &[
            "--seat",
            &format!("w1/w1:t1/{P1}@{b_tag}"),
            "--body",
            "over there",
        ],
    );
    cluster.wait_until("b to hold the message", crate::WAIT, || {
        support::inbox_ids(&cluster, &b)
            .contains(&message)
            .then_some(())
    });
    // An exact seat ID qualified by its own machine, and by another one.
    assert_eq!(
        inspect(&cluster, &a, &format!("{}@{b_tag}", b.id)),
        Ok(b.id.clone())
    );
    let err = inspect(&cluster, &a, &format!("{}@alpha", b.id)).unwrap_err();
    assert!(
        err.contains(&format!("lives on {b_tag} beta, not alpha")),
        "{err}"
    );
    // An unseated pane on beta: refused, nothing allocated there.
    let err = inspect(&cluster, &a, &format!("{P2}@{b_tag}")).unwrap_err();
    assert!(err.contains("no seat at"), "{err}");
    // An unknown machine.
    let err = inspect(&cluster, &a, &format!("{P1}@nowhere")).unwrap_err();
    assert!(err.contains("no machine nowhere; known:"), "{err}");
    // Our own machine resolves locally (by tag: `w1:p1@alpha` is also a
    // local label, below).
    let a_tag = tag_of(&cluster, 0, 0);
    assert_eq!(
        inspect(&cluster, &a, &format!("{P1}@{a_tag}")),
        Ok(a.id.clone())
    );
    // beta.lab has no addr.resolve.v1: refused naming the cap, even though
    // the same path names a local pane.
    let err = inspect(&cluster, &a, &format!("{P1}@beta.lab")).unwrap_err();
    assert!(err.contains("needs addr.resolve.v1"), "{err}");
    assert!(err.contains(r"\@"), "the local escape hint: {err}");
    // A local `@` label whose suffix names no machine keeps its local
    // meaning (that pane holds no seat).
    let err = inspect(&cluster, &a, "user@nowhere").unwrap_err();
    assert!(err.contains("w1:p5"), "{err}");
    // A local label that is also a live remote address: both readings are
    // listed and nothing is picked.
    let err = inspect(&cluster, &a, &format!("w1:p1@{}", &b_uuid[..8])).unwrap_err();
    assert!(err.contains("2 readings"), "{err}");
    assert!(err.contains(r"\@"), "{err}");
    // The same with our own machine: the label and the plain location name
    // different local panes.
    let err = inspect(&cluster, &a, "w1:p1@alpha").unwrap_err();
    assert!(err.contains("2 readings"), "{err}");
    // A local `@` label narrowed by --space whose machine answers no match:
    // resolved locally, as before remote addresses (that pane holds no seat).
    let (code, value, stderr) = cluster.run_as(
        Some(&a),
        0,
        &[
            "inbox",
            "--seat",
            &format!("ci@{}", &b_uuid[..9]),
            "--space",
            "w1",
        ],
    );
    assert_ne!(code, 0);
    assert!(stderr.contains("w1:p8"), "{stderr}{value}");
    // Its exact seat ID still works.
    assert_eq!(inspect(&cluster, &a, &c.id), Ok(c.id.clone()));
    // beta unreachable: refused, the exact seat ID still works.
    cluster.partition(0, 1);
    cluster.wait_until("node 0's link to beta to drop", crate::WAIT, || {
        inspect(&cluster, &a, &format!("w1/w1:t1/{P1}@{b_tag}"))
            .err()
            .filter(|e| e.contains("unreachable"))
    });
    assert_eq!(inspect(&cluster, &a, &b.id), Ok(b.id.clone()));
    cluster.heal(0, 1);
}

// Kills: a handoff into a seat on another node refused (the phase 1
// refusal), the eligibility check skipped while the home node is up, and a
// sleeping home node refusing a handoff to an exact remote seat ID instead
// of authoring it and saying the check used replicated state only (N15).
#[test]
fn handoff_into_a_remote_seat_is_authored_by_the_sender() {
    let mut cluster = Cluster::start(
        "mn-remote-handoff",
        crate::BIN,
        2,
        ClusterOptions {
            links: vec![(0, 1)],
            ..Default::default()
        },
    );
    cluster.add_host(0, &[P1]);
    cluster.add_host(1, &[P1]);
    // Handoff needs a daemon started with its host (guarded bootstrap).
    cluster.stop_node(0);
    cluster.start_node(0);
    let a = cluster.seat(0, P1);
    let b = cluster.seat(1, P1);
    support::wait_seats(&cluster, 0, 2);
    cluster.wait_converged(crate::WAIT);
    let handoff = |task: &str| {
        cluster.run_as(
            Some(&a),
            0,
            &["handoff", "--existing", "--seat", &b.id, "--", task],
        )
    };
    let (code, value, stderr) = handoff("first task");
    assert_eq!(code, 0, "{stderr}{value}");
    assert!(!stderr.contains("replicated state only"), "{stderr}");
    let delivered = |text: &str| {
        let data = cluster.ok_as(Some(&b), 1, &["inbox"]);
        data["items"]
            .as_array()
            .is_some_and(|items| items.iter().any(|i| i.to_string().contains(text)))
            .then_some(())
    };
    cluster.wait_until("b to get the first task", crate::WAIT, || {
        delivered("first task")
    });
    // The home node asleep: authored anyway, with the note.
    cluster.partition(0, 1);
    let (code, value, stderr) = cluster.wait_until("the link to drop", crate::WAIT, || {
        let out = handoff("second task");
        (out.0 != 0 || out.2.contains("replicated state only")).then_some(out)
    });
    assert_eq!(code, 0, "{stderr}{value}");
    assert!(
        stderr.contains("checked against replicated state only"),
        "{stderr}"
    );
    cluster.heal(0, 1);
    cluster.wait_until("b to get the second task", crate::WAIT, || {
        delivered("second task")
    });
}

/// The stand-in for the launching node's children (`internal
/// peer-tab-create`, `launch`): records each call's argv and environment in
/// the log dir, waits while `hold` exists, and answers like the real ones.
const LAUNCH_STUB: &str = r#"#!/bin/sh
log="$HT_TEST_PEER_LAUNCH_LOG"
n=$(ls "$log" | grep -c '^call-' )
{ printf '%s\n' "$@"; echo '--env--'; env; } > "$log/call-$n"
while [ -e "$log/hold" ]; do sleep 0.1; done
for a in "$@"; do
  case "$a" in
    peer-tab-create) echo '{"pane":"w1:p9","tab":"w1:t9"}'; exit 0;;
    launch) printf '{"launch":{"outcome":"started","seat":"%s"}}\n' "$(cat "$log/seat")"; exit 0;;
  esac
done
exit 1
"#;

fn calls(log: &std::path::Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(log)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().into_string().unwrap())
        .filter(|n| n.starts_with("call-"))
        .collect();
    names.sort();
    names
        .iter()
        .map(|n| std::fs::read_to_string(log.join(n)).unwrap())
        .collect()
}

// Kills: a launch on another machine refused or run through the asking
// node's own Herdr, the launching node's child inheriting a caller pane or
// harness session (stray attribution), a retry after a dropped link
// launching twice, a held target launched, a new tab not labelled with the
// requesting node, and an older peer not refused (N15).
#[test]
fn remote_launch_runs_on_the_target_machine_once() {
    use std::os::unix::fs::PermissionsExt;

    let log = herdr_threads::test_support::scratch::temp_path(format!(
        "ht-peer-launch-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&log).unwrap();
    let stub = log.join("stub.sh");
    std::fs::write(&stub, LAUNCH_STUB).unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o700)).unwrap();
    let calls_dir = log.join("calls");
    std::fs::create_dir_all(&calls_dir).unwrap();
    let mut env = vec![
        (
            1,
            "HT_TEST_PEER_LAUNCH_STUB".to_owned(),
            stub.display().to_string(),
        ),
        (
            1,
            "HT_TEST_PEER_LAUNCH_LOG".to_owned(),
            calls_dir.display().to_string(),
        ),
        // A stray caller pane and session in the daemon's own environment.
        (1, "HERDR_PANE_ID".to_owned(), "w1:p1".to_owned()),
        (1, "CODEX_SESSION_ID".to_owned(), "stray".to_owned()),
    ];
    env.push((
        2,
        "HT_TEST_WITHHOLD_CAPS".to_owned(),
        "peer.launch.v1".to_owned(),
    ));
    let mut cluster = Cluster::start(
        "mn-remote-launch",
        crate::BIN,
        3,
        ClusterOptions {
            links: vec![(0, 1), (0, 2)],
            env,
            ..Default::default()
        },
    );
    cluster.add_host(0, &[P1]);
    cluster.add_host(1, &[P1, P2, P3]);
    cluster.add_host(2, &[P1]);
    // Handoff needs a daemon started with its host (guarded bootstrap).
    cluster.stop_node(0);
    cluster.start_node(0);
    let a = cluster.seat(0, P1);
    let b = cluster.seat(1, P1);
    let _c = cluster.seat(2, P1);
    support::wait_seats(&cluster, 0, 3);
    cluster.wait_converged(crate::WAIT);
    // The stub answers with b's seat: a seat node 0 holds as node 1's.
    std::fs::write(calls_dir.join("seat"), &b.id).unwrap();
    let b_tag = tag_of(&cluster, 0, 1);
    let c_tag = tag_of(&cluster, 0, 2);
    let a_tag = tag_of(&cluster, 0, 0);
    // The asker gives up after 8 s here (120 s in production), so a rerun runs.
    let launch = |pane: &str, arg: &str| {
        cluster.text_as_env(
            Some(&a),
            0,
            &[
                "launch",
                "--pane",
                &format!("{pane}@{b_tag}"),
                "--kind",
                "claude",
                "--",
                arg,
            ],
            &[("HT_TEST_REMOTE_LAUNCH_TIMEOUT_MS", "8000")],
        )
    };

    // A launch into a pane on node 1, run there as its own `launch`.
    let (code, stdout, stderr) = launch(P2, "--first");
    assert_eq!(code, 0, "{stderr}{stdout}");
    assert!(
        stdout.contains(&format!("launched seat={}", b.id)),
        "{stdout}"
    );
    let first = calls(&calls_dir);
    assert_eq!(first.len(), 1, "{first:?}");
    let (argv, env) = first[0].split_once("--env--").unwrap();
    assert!(
        argv.contains("launch\n--pane\nw1:p2\n--kind\nclaude\n--\n--first"),
        "{argv}"
    );
    assert!(!env.contains("HERDR_PANE_ID"), "{env}");
    assert!(!env.contains("CODEX_SESSION_ID"), "{env}");
    assert!(env.contains("HOME="), "{env}");

    // A held target is refused on node 1, before anything runs there.
    // An open hold on w1:p3, kept by an unresolved seat on the same host
    // (the daemon lifts a host's holds only once none is left).
    cluster.write_sql(
        1,
        "INSERT INTO recovery_holds(host_id,target_id,baseline_boot,baseline_epoch,reason) \
         SELECT host_id,'w1:p3','fixture',0,'fixture' FROM herdr_hosts LIMIT 1",
    );
    cluster.write_sql(
        1,
        "INSERT INTO seats(id,instance_id,state,unresolved_reason,role,generation,created_at,home_node,is_local,host_id) \
         SELECT 'sHELDfixture','' || (SELECT id FROM local_node),'unresolved','other','native',1,0,\
         (SELECT id FROM local_node),1,host_id FROM herdr_hosts LIMIT 1",
    );
    let (code, stdout, stderr) = launch(P3, "--held");
    assert_ne!(code, 0, "{stdout}");
    assert!(stderr.contains("held for recovery"), "{stderr}");
    assert_eq!(calls(&calls_dir).len(), 1, "nothing ran for the held pane");

    // A refusal is recorded: no such pane there.
    let (code, _, stderr) = launch("nosuchpane", "--x");
    assert_ne!(code, 0);
    assert!(
        stderr.contains("refused the launch: no such pane here"),
        "{stderr}"
    );

    // The asker gives up while node 1 is unreachable mid-launch; a rerun of
    // the same command reuses the request id and gets that launch's outcome:
    // the child ran once.
    std::fs::write(calls_dir.join("hold"), b"").unwrap();
    let (code, _, stderr) = std::thread::scope(|scope| {
        let run = scope.spawn(|| launch(P2, "--second"));
        cluster.wait_until("node 1 to start the launch", crate::WAIT, || {
            (calls(&calls_dir).len() == 2).then_some(())
        });
        cluster.partition(0, 1);
        run.join().unwrap()
    });
    assert_ne!(code, 0);
    assert!(stderr.contains("rerun to resume"), "{stderr}");
    cluster.heal(0, 1);
    std::fs::remove_file(calls_dir.join("hold")).unwrap();
    let (code, stdout, stderr) =
        cluster.wait_until("the rerun to get the outcome", crate::WAIT, || {
            let out = launch(P2, "--second");
            (out.0 == 0).then_some(out)
        });
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.contains(&format!("launched seat={}", b.id)),
        "{stdout}"
    );
    assert_eq!(calls(&calls_dir).len(), 2, "one launch for one request");

    // A new tab whose cwd does not exist there: refused on node 1.
    let (code, _, stderr) = cluster.text_as(
        Some(&a),
        0,
        &[
            "handoff",
            "--new-tab",
            "x",
            "--space",
            &format!("w1@{b_tag}"),
            "--cwd",
            "/nonexistent/herdr-threads-test",
            "--kind",
            "claude",
            "--",
            "task",
        ],
    );
    assert_ne!(code, 0);
    assert!(
        stderr.contains("needs an existing absolute cwd here"),
        "{stderr}"
    );

    // A remote handoff into a new tab, labelled with the requesting node; the
    // task then reaches the launched seat (the stub names b's).
    let (code, stdout, stderr) = cluster.text_as(
        Some(&a),
        0,
        &[
            "handoff",
            "--new-tab",
            "review",
            "--space",
            &format!("w1@{b_tag}"),
            "--cwd",
            "/tmp",
            "--kind",
            "claude",
            "--",
            "look at this",
        ],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    let tab_call = calls(&calls_dir)
        .into_iter()
        .find(|c| c.contains("peer-tab-create"))
        .unwrap_or_else(|| panic!("no tab created: {stdout}{stderr}"));
    assert!(
        tab_call.contains(&format!("--label\nreview [from {a_tag}]")),
        "{tab_call}"
    );
    cluster.wait_until("b to get the handed-off task", crate::WAIT, || {
        let data = cluster.ok_as(Some(&b), 1, &["inbox"]);
        data["items"]
            .as_array()
            .is_some_and(|items| items.iter().any(|i| i.to_string().contains("look at this")))
            .then_some(())
    });

    // peer list on node 1 shows node 0's launches.
    let (_, peers, _) = cluster.text_as(None, 1, &["peer", "list"]);
    assert!(peers.contains("peer_launches=6 last=launched:"), "{peers}");

    // An older peer without peer.launch.v1: refused, nothing local.
    let (code, _, stderr) = cluster.text_as(
        Some(&a),
        0,
        &[
            "launch",
            "--pane",
            &format!("{P1}@{c_tag}"),
            "--kind",
            "claude",
        ],
    );
    assert_ne!(code, 0);
    assert!(stderr.contains("needs peer.launch.v1"), "{stderr}");
}
