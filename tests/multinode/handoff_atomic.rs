//! Atomic remote handoff, end to end (ht-8lem): `handoff --pane P@M` and `--new-tab --space S@M`
//! stage the task on the target before the harness starts, the sender's ledger follows the
//! request to `linked`, and every refusal, kill, partition and tamper path leaves the facts and
//! the ledger as the spec says.
//!
//! The target node's launch children are `remote_addresses::LAUNCH_STUB`. For a handoff plan it
//! execs the real `internal peer-handoff-launch` child, which preflights against the fake Herdr
//! pane, stages through the daemon and, in place of starting a harness, records the launch
//! request it would have submitted (`HT_TEST_HANDOFF_LAUNCH_RECORD`; `.hold` and `.fail` files
//! beside it pause or fail the launch after staging).

use crate::remote_addresses::{LAUNCH_STUB, calls};
use crate::support::{self, P1, P2, P3};
use herdr_threads::protocol::ids::{SeatId, direct_thread_id};
use herdr_threads::test_support::multinode::{Cluster, ClusterOptions, Seat};
use std::path::{Path, PathBuf};
use std::time::Duration;

const P4: &str = "w1:p4";
const TASK: &str = "port the parser to the new lexer";
const SUMMARY: &str = "parser port";

/// A two-node cluster whose node 1 launches through the stub, and the requesting seat on node 0.
struct Rig {
    cluster: Cluster,
    /// The requesting seat (node 0).
    a: Seat,
    /// A seat of node 1 (pane P1).
    b: Seat,
    /// Node 1's tag, as node 0 knows it.
    tag: String,
    /// Node 1's stub log directory.
    log: PathBuf,
    /// A directory on node 1 to pass as `--profile-dir`.
    profile: PathBuf,
}

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

impl Rig {
    fn new(label: &str, extra_env: &[(usize, &str, &str)]) -> Rig {
        use std::os::unix::fs::PermissionsExt;
        let root = herdr_threads::test_support::scratch::temp_path(format!(
            "ht-handoff-atomic-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let log = root.join("calls");
        let profile = root.join("profile");
        std::fs::create_dir_all(&log).unwrap();
        std::fs::create_dir_all(&profile).unwrap();
        let stub = root.join("stub.sh");
        std::fs::write(&stub, LAUNCH_STUB).unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(log.join("real"), b"").unwrap();
        let mut env = vec![
            (
                1,
                "HT_TEST_PEER_LAUNCH_STUB".to_owned(),
                stub.display().to_string(),
            ),
            (
                1,
                "HT_TEST_PEER_LAUNCH_LOG".to_owned(),
                log.display().to_string(),
            ),
            (1, "HT_TEST_REAL_BIN".to_owned(), crate::BIN.to_owned()),
            (
                1,
                "HT_TEST_HANDOFF_LAUNCH_RECORD".to_owned(),
                log.join("launch-record").display().to_string(),
            ),
        ];
        env.extend(
            extra_env
                .iter()
                .map(|(n, k, v)| (*n, (*k).to_owned(), (*v).to_owned())),
        );
        let mut cluster = Cluster::start(
            label,
            crate::BIN,
            2,
            ClusterOptions {
                links: vec![(0, 1)],
                env,
                ..Default::default()
            },
        );
        cluster.add_host(0, &[P1]);
        cluster.add_host(1, &[P1, P2, P3, P4]);
        // Handoff needs a daemon started with its host (guarded bootstrap).
        cluster.stop_node(0);
        cluster.start_node(0);
        let a = cluster.seat(0, P1);
        let b = cluster.seat(1, P1);
        support::wait_seats(&cluster, 0, 2);
        support::wait_seats(&cluster, 1, 2);
        cluster.wait_converged(crate::WAIT);
        let tag = tag_of(&cluster, 0, 1);
        Rig {
            cluster,
            a,
            b,
            tag,
            log,
            profile,
        }
    }

    /// `handoff` arguments into `target` (the target flags), with the task `TASK`.
    fn args(&self, target: &[&str], extra: &[&str]) -> Vec<String> {
        let mut args = vec!["handoff".to_owned()];
        args.extend(target.iter().map(|s| (*s).to_owned()));
        args.extend(["--kind", "claude", "--summary", SUMMARY].map(str::to_owned));
        // A pane cannot change the profile directory: only a new tab carries `--profile-dir`.
        if target.first() != Some(&"--pane") {
            args.push("--profile-dir".to_owned());
            args.push(self.profile.display().to_string());
        }
        args.extend(extra.iter().map(|s| (*s).to_owned()));
        args.extend(["--".to_owned(), TASK.to_owned()]);
        args
    }

    /// `--pane PANE@node1`.
    fn pane(&self, pane: &str) -> Vec<String> {
        vec!["--pane".to_owned(), format!("{pane}@{}", self.tag)]
    }

    /// Runs the CLI as the requesting seat, giving up after `timeout_ms`.
    fn run(&self, args: &[String], timeout_ms: &str) -> (i32, String, String) {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.cluster.text_as_env(
            Some(&self.a),
            0,
            &refs,
            &[("HT_TEST_REMOTE_LAUNCH_TIMEOUT_MS", timeout_ms)],
        )
    }

    fn handoff_pane(&self, pane: &str, extra: &[&str], timeout_ms: &str) -> (i32, String, String) {
        let target = self.pane(pane);
        let refs: Vec<&str> = target.iter().map(String::as_str).collect();
        self.run(&self.args(&refs, extra), timeout_ms)
    }

    fn record(&self) -> PathBuf {
        self.log.join("launch-record")
    }

    /// The `(state, answer)` of node 0's ledger rows.
    fn ledger(&self) -> Vec<(String, String)> {
        self.cluster
            .rows(
                0,
                "SELECT state, ifnull(answer,'') FROM handoff_obligations ORDER BY created_at",
            )
            .into_iter()
            .map(|r| (r[0].clone(), r[1].clone()))
            .collect()
    }

    /// The launch record once it exists and parses: the launcher writes it with a plain
    /// `std::fs::write` after the CLI may already have exited, so a present file can be partial.
    fn wait_record(&self) -> serde_json::Value {
        self.cluster
            .wait_until("node 1's launch record", crate::WAIT, || {
                std::fs::read_to_string(self.record())
                    .ok()
                    .and_then(|text| serde_json::from_str(&text).ok())
            })
    }

    fn wait_ledger(&self, state: &str) {
        self.cluster.wait_until(
            &format!("node 0's ledger row to be {state}"),
            crate::WAIT,
            || (self.ledger().first().map(|r| r.0.as_str()) == Some(state)).then_some(()),
        );
    }

    /// The node-1 messages carrying `TASK`: `(actor, deadline_node, origin_node)` rows.
    fn task_rows(&self, node: usize) -> Vec<Vec<String>> {
        self.cluster.rows(
            node,
            &format!(
                "SELECT actor_seat_id, ifnull(deadline_node,''), ifnull(origin_node,'') \
                 FROM messages WHERE body='{TASK}'"
            ),
        )
    }

    fn launch_requests(&self) -> usize {
        calls(&self.log.join("."))
            .iter()
            .filter(|c| c.contains("peer-handoff-launch"))
            .count()
    }
}

/// The `key=value` fields of a `handoff seat=.. pane=.. thread=.. message=..` result line.
fn field<'a>(line: &'a str, key: &str) -> &'a str {
    line.split_whitespace()
        .find_map(|w| w.strip_prefix(&format!("{key}=")))
        .unwrap_or_else(|| panic!("no {key} in {line:?}"))
}

/// The prompt, the profile directory (`None`: a pane carries none) and the harness the recorded
/// launch request carries.
fn assert_record(record: &serde_json::Value, sender: &Seat, profile: Option<&Path>) {
    let argv = record["argv"].to_string();
    assert!(
        argv.contains(SUMMARY),
        "the prompt carries the summary: {argv}"
    );
    assert!(
        argv.contains(&format!("from: {}", sender.id)),
        "the prompt carries the from line: {argv}"
    );
    let context = &record["directory_context"];
    let Some(profile) = profile else {
        assert!(
            context.is_null(),
            "a pane carries no directory context: {context}"
        );
        return;
    };
    assert_eq!(
        context["profile_dir"]
            .as_str()
            .map(PathBuf::from)
            .as_deref(),
        Some(profile.canonicalize().unwrap().as_path()),
        "{context}"
    );
    let env = context["env"].to_string();
    assert!(
        env.contains(profile.canonicalize().unwrap().to_str().unwrap()),
        "the profile env points at the directory: {env}"
    );
}

// Kills: a handoff whose task reaches the target after the harness starts (the old two-step
// flow), a prompt without the summary or the from line, a pane launch that carries a profile
// directory (a pane cannot change it), a sender block whose deadline owner is not the sender node, a ledger row that
// never links, and a second pair thread when the sender's pair and the target's `thread.direct`
// cross.
#[test]
fn handoff_pane_atomic() {
    let rig = Rig::new("mn-handoff-pane", &[]);
    let cluster = &rig.cluster;
    let sender_node = cluster.node_uuid(0).to_string();
    // Hold the launch after staging: the task must already be on node 1.
    std::fs::write(rig.log.join("launch-record.hold"), b"").unwrap();
    let (code, stdout, stderr) = std::thread::scope(|scope| {
        let run = scope.spawn(|| rig.handoff_pane(P2, &["--deadline", "600"], "60000"));
        cluster.wait_until("the task to be staged on node 1", crate::WAIT, || {
            (!rig.task_rows(1).is_empty()).then_some(())
        });
        assert!(
            !rig.record().exists(),
            "the harness launch must wait for staging, not the reverse"
        );
        let staged = rig.task_rows(1);
        assert_eq!(staged.len(), 1, "{staged:?}");
        assert_eq!(staged[0][0], rig.a.id, "authored as the sender seat");
        assert_eq!(
            staged[0][1], sender_node,
            "the sender node owns the deadline"
        );
        assert_eq!(
            staged[0][2],
            cluster.node_uuid(1).to_string(),
            "origin is the target"
        );
        std::fs::remove_file(rig.log.join("launch-record.hold")).unwrap();
        run.join().unwrap()
    });
    assert_eq!(code, 0, "{stdout}{stderr}");
    let seat = field(&stdout, "seat");
    let thread = field(&stdout, "thread");
    let message = field(&stdout, "message");
    // The link may beat the answer: a ledger `linked` result carries no pane.
    if stdout.contains(" pane=") {
        assert_eq!(field(&stdout, "pane"), format!("{P2}@{}", rig.tag));
    }
    assert_eq!(
        thread,
        direct_thread_id(
            &SeatId::new(rig.a.id.clone()),
            &SeatId::new(seat.to_owned())
        )
        .unwrap()
        .as_str()
    );

    // The child ran once, with the request and the pane; its launch carries prompt and profile.
    // The record is written after the stub logged its call, and after the CLI may have exited.
    let record = rig.wait_record();
    let launches = calls(&rig.log);
    assert_eq!(launches.len(), 1, "{launches:?}");
    assert!(
        launches[0].contains("internal\npeer-handoff-launch\n--request=")
            && launches[0].contains(&format!("--pane={P2}\n")),
        "{}",
        launches[0]
    );
    assert_record(&record, &rig.a, None);

    // The prompt was frozen on the target's row; the ledger links; the message is on both nodes.
    assert!(
        cluster
            .scalar(1, "SELECT handoff_prompt FROM peer_requests", &[])
            .is_some_and(|p| p.contains(SUMMARY) && p.contains(&rig.a.id))
    );
    rig.wait_ledger("linked");
    cluster.wait_until("the message to reach node 0", crate::WAIT, || {
        (rig.task_rows(0).len() == 1).then_some(())
    });
    assert_eq!(
        cluster
            .scalar(0, "SELECT message FROM handoff_obligations", &[])
            .as_deref(),
        Some(message)
    );
    cluster.wait_converged(crate::WAIT);
    // One pair thread, whichever node opened it first.
    for node in 0..2 {
        assert_eq!(
            cluster
                .scalar(
                    node,
                    &format!("SELECT count(*) FROM threads WHERE id='{thread}'"),
                    &[]
                )
                .as_deref(),
            Some("1"),
            "node {node} holds the pair thread once"
        );
        assert_eq!(
            cluster
                .scalar(
                    node,
                    "SELECT count(*) FROM threads WHERE topic NOT LIKE 'seats %' AND id!=?1 \
                     AND id IN (SELECT thread_id FROM messages WHERE kind='ordinary')",
                    &[&thread]
                )
                .as_deref(),
            Some("0"),
            "node {node}: no other thread carries a message"
        );
    }
}

// Kills: a new-tab handoff that skips the staging child, a tab not labelled with the sender, and
// a launch into the created pane that loses the prompt or the profile directory, and a tab
// created without the profile env.
#[test]
fn handoff_new_tab_atomic() {
    let rig = Rig::new("mn-handoff-tab", &[]);
    std::fs::write(rig.log.join("pane"), P3).unwrap();
    let (code, stdout, stderr) = rig.run(
        &rig.args(
            &[
                "--new-tab",
                "review",
                "--space",
                &format!("w1@{}", rig.tag),
                "--cwd",
                "/tmp",
            ],
            &[],
        ),
        "60000",
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    if stdout.contains(" pane=") {
        assert_eq!(field(&stdout, "pane"), format!("{P3}@{}", rig.tag));
    }
    let record = rig.wait_record();
    let all = calls(&rig.log);
    assert_eq!(all.len(), 2, "tab create, then the staging child: {all:?}");
    assert!(all[0].contains("peer-target-create"), "{}", all[0]);
    assert!(
        all[0].contains(&format!(
            "--label=review [from {}]",
            tag_of(&rig.cluster, 0, 0)
        )),
        "{}",
        all[0]
    );
    assert!(all[1].contains("peer-handoff-launch"), "{}", all[1]);
    assert_record(&record, &rig.a, Some(&rig.profile));
    // The frozen context rides into the tab create (its env is what the tab receives).
    let profile = rig.profile.canonicalize().unwrap();
    let flag = all[0]
        .lines()
        .find_map(|l| l.strip_prefix("--directory-context="))
        .unwrap_or_else(|| panic!("the tab create carries the context: {}", all[0]));
    let context: serde_json::Value = serde_json::from_str(flag).unwrap();
    assert_eq!(context["cwd"], "/tmp");
    assert!(
        context["env"]
            .as_object()
            .unwrap()
            .values()
            .any(|v| v.as_str() == profile.to_str()),
        "{context}"
    );
    assert_eq!(rig.task_rows(1).len(), 1);
    rig.wait_ledger("linked");
}

// Kills: a `peer-target-create` child that drops the frozen profile env (the tab's shell, and so
// the agent started in its root pane, would run on the default root while the CLI reports
// started), an env sent without its context or a cwd other than the plan's, and a bare tab that
// receives an env it was never given. Asserted at Herdr's `tab.create`, the call that carries
// the environment of the tab's shell.
#[test]
fn peer_target_create_hands_the_profile_env_to_the_tab() {
    use herdr_threads::test_support::{fake_herdr::FakeHerdr, scratch, spawn};
    let socket = scratch::temp_path(format!("ht-ptc-{}.sock", std::process::id()));
    let fake = FakeHerdr::start("pt", &socket, &["w1:p1"]);
    let dir = scratch::temp_path(format!("ht-ptc-dirs-{}", std::process::id()));
    let (profile, cwd) = (dir.join("profile"), dir.join("cwd"));
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    let context = serde_json::json!({
        "profile_dir": profile,
        "cwd": cwd,
        "env": {"PROFILE_ROOT": profile},
    });
    let run = |extra: Option<String>| {
        let mut command = spawn::command(crate::BIN);
        command.env("HOME", &dir).env("XDG_STATE_HOME", &dir);
        command.args(["--json", "--host-endpoint"]);
        command.arg(&socket);
        command.args([
            "internal",
            "peer-target-create",
            "--workspace=w1",
            "--label=x",
        ]);
        command.arg(format!("--cwd={}", cwd.display()));
        command.args(extra);
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("\"pane\""), "{stdout}");
    };
    run(Some(format!("--directory-context={context}")));
    run(None);
    let tabs = fake.tabs_created();
    assert_eq!(tabs.len(), 2, "{tabs:?}");
    assert_eq!(tabs[0]["env"], serde_json::json!({"PROFILE_ROOT": profile}));
    assert_eq!(tabs[0]["cwd"], serde_json::json!(cwd));
    assert_eq!(tabs[1]["env"], serde_json::json!({}));
    drop(fake);
    let _ = std::fs::remove_dir_all(&dir);
}

// Kills: a bare `launch --pane P@M` turned into a handoff (a ledger row, a signed message, the
// staging child) and a bare launch that stops reaching the plain `launch` child.
#[test]
fn bare_launch_unchanged() {
    let rig = Rig::new("mn-handoff-bare", &[]);
    std::fs::write(rig.log.join("seat"), &rig.b.id).unwrap();
    std::fs::remove_file(rig.log.join("real")).unwrap();
    let before = (rig.cluster.kinds(0), rig.cluster.kinds(1));
    let (code, stdout, stderr) = rig.cluster.text_as_env(
        Some(&rig.a),
        0,
        &[
            "launch",
            "--pane",
            &format!("{P2}@{}", rig.tag),
            "--kind",
            "claude",
        ],
        &[("HT_TEST_REMOTE_LAUNCH_TIMEOUT_MS", "20000")],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("launched seat="), "{stdout}");
    let all = calls(&rig.log);
    assert_eq!(all.len(), 1, "{all:?}");
    assert!(
        all[0].contains("launch\n--pane=w1:p2\n--kind=claude")
            && !all[0].contains("peer-handoff-launch"),
        "{}",
        all[0]
    );
    assert_eq!(rig.ledger(), Vec::<(String, String)>::new());
    assert_eq!((rig.cluster.kinds(0), rig.cluster.kinds(1)), before);
    assert!(!rig.record().exists());
}

/// Waits until `calls` shows the target started its launch child.
fn wait_child_started(rig: &Rig) {
    rig.cluster
        .wait_until("node 1 to start the launch child", crate::WAIT, || {
            (rig.launch_requests() >= 1).then_some(())
        });
}

// Kills: a target that waits for the sender CLI (a killed CLI strands the task), a request that
// is not replayed after the sender's daemon restarts, and a ledger row that stays `sent` once
// the message replicates.
#[test]
fn sender_cli_killed_or_daemon_restarted() {
    use herdr_threads::test_support::spawn::SpawnOwned;
    let mut rig = Rig::new("mn-handoff-kill", &[]);
    // Case 1: the CLI is killed right after the request reached the target.
    std::fs::write(rig.log.join("hold"), b"").unwrap();
    let mut command = rig.cluster.pane_command(&rig.a);
    command.args(rig.args(
        &rig.pane(P2).iter().map(String::as_str).collect::<Vec<_>>(),
        &[],
    ));
    command.env("HT_TEST_REMOTE_LAUNCH_TIMEOUT_MS", "60000");
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = command.spawn_owned().unwrap();
    wait_child_started(&rig);
    child.kill().unwrap();
    let _ = child.wait();
    std::fs::remove_file(rig.log.join("hold")).unwrap();
    rig.cluster
        .wait_until("the task on node 1", crate::WAIT, || {
            (rig.task_rows(1).len() == 1 && rig.record().exists()).then_some(())
        });
    rig.cluster
        .wait_until("the task on node 0", crate::WAIT, || {
            (rig.task_rows(0).len() == 1).then_some(())
        });
    rig.wait_ledger("linked");
    let _ = std::fs::remove_file(rig.record());

    // Case 2: the sender's daemon stops right after the request; the target proceeds alone and
    // the restarted daemon links the row. A different pane, so a second request.
    let before = rig.ledger().len();
    std::fs::write(rig.log.join("hold"), b"").unwrap();
    let mut command = rig.cluster.pane_command(&rig.a);
    command.args(rig.args(
        &rig.pane(P3).iter().map(String::as_str).collect::<Vec<_>>(),
        &[],
    ));
    command.env("HT_TEST_REMOTE_LAUNCH_TIMEOUT_MS", "60000");
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = command.spawn_owned().unwrap();
    rig.cluster
        .wait_until("node 1 to start the second launch", crate::WAIT, || {
            (rig.launch_requests() >= 2).then_some(())
        });
    rig.cluster.stop_node(0);
    let _ = child.kill();
    let _ = child.wait();
    std::fs::remove_file(rig.log.join("hold")).unwrap();
    rig.cluster
        .wait_until("the second launch on node 1", crate::WAIT, || {
            rig.record().exists().then_some(())
        });
    rig.cluster.start_node(0);
    rig.cluster
        .wait_until("both ledger rows linked", crate::WAIT, || {
            let ledger = rig.ledger();
            (ledger.len() == before + 1 && ledger.iter().all(|r| r.0 == "linked")).then_some(())
        });
    assert_eq!(
        rig.task_rows(1).len(),
        2,
        "one task per request on the target"
    );
    rig.cluster
        .wait_until("both tasks on node 0", crate::WAIT, || {
            (rig.task_rows(0).len() == 2).then_some(())
        });
    let _ = &mut rig;
}

// Kills: a re-send of a running request that launches a second agent or creates a second seat,
// and a CLI rerun that stops reusing the request id.
#[test]
fn resend_while_running_launches_once() {
    let rig = Rig::new("mn-handoff-resend", &[]);
    std::fs::write(rig.log.join("hold"), b"").unwrap();
    // The first poll gives up while the child still holds; the rerun re-sends the same request.
    let (code, _, stderr) = rig.handoff_pane(P2, &[], "3000");
    assert_ne!(code, 0);
    assert!(stderr.contains("still pending"), "{stderr}");
    let (code, _, stderr) = rig.handoff_pane(P2, &[], "3000");
    assert_ne!(code, 0);
    assert!(stderr.contains("still pending"), "{stderr}");
    std::fs::remove_file(rig.log.join("hold")).unwrap();
    let (code, stdout, stderr) =
        rig.cluster
            .wait_until("the rerun to get the outcome", crate::WAIT, || {
                let out = rig.handoff_pane(P2, &[], "20000");
                (out.0 == 0).then_some(out)
            });
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert_eq!(rig.launch_requests(), 1, "one launch child for one request");
    assert_eq!(
        rig.cluster
            .scalar(1, "SELECT count(*) FROM peer_requests", &[])
            .as_deref(),
        Some("1")
    );
    assert_eq!(rig.ledger().len(), 1);
    assert_eq!(rig.task_rows(1).len(), 1, "the task was staged once");
    // P2's seat is the only seat the request made on node 1 (P1's was there before).
    assert_eq!(
        rig.cluster
            .scalar(1, "SELECT count(*) FROM seats WHERE is_local=1", &[])
            .as_deref(),
        Some("2")
    );
}

// Kills: a target that fails after staging and drops the staged message from the failure (the
// sender would believe nothing was authored), and a ledger row that goes terminal instead of
// keeping the obligation until the message replicates. (The CLI text `; message <m> stands in
// thread <t>` is asserted by `fail_after_staging_prints_the_staged_message`; here the link races
// the answer.)
#[test]
fn fail_after_staging_keeps_the_obligation() {
    let rig = Rig::new("mn-handoff-fail-staged", &[]);
    let cluster = &rig.cluster;
    std::fs::write(rig.log.join("hold"), b"").unwrap();
    std::fs::write(rig.log.join("launch-record.fail"), b"").unwrap();
    let (code, stdout, stderr) = std::thread::scope(|scope| {
        let run = scope.spawn(|| rig.handoff_pane(P2, &["--deadline", "600"], "60000"));
        wait_child_started(&rig);
        // The sender cannot hear the failure, nor receive the staged message, until it heals.
        cluster.partition(0, 1);
        std::fs::remove_file(rig.log.join("hold")).unwrap();
        cluster.wait_until("the launch to fail on node 1", crate::WAIT, || {
            (cluster
                .scalar(1, "SELECT state FROM peer_requests", &[])
                .as_deref()
                == Some("failed"))
            .then_some(())
        });
        assert_eq!(rig.ledger()[0].0, "sent", "the sender heard nothing");
        assert_eq!(
            rig.task_rows(1).len(),
            1,
            "the staged facts stand on the target"
        );
        assert!(rig.task_rows(0).is_empty());
        cluster.heal(0, 1);
        run.join().unwrap()
    });
    let _ = (code, stdout, stderr);
    assert!(!rig.record().exists(), "no harness was started");
    cluster.wait_until("the staged message to reach node 0", crate::WAIT, || {
        (rig.task_rows(0).len() == 1).then_some(())
    });
    rig.wait_ledger("linked");
}

// Kills: a CLI that hides the staged message when the target fails after staging (the sender
// would believe nothing was authored). The ledger's link is held off (its message column is
// pointed elsewhere) so the failed answer, not the replicated message, is what the CLI hears.
#[test]
fn fail_after_staging_prints_the_staged_message() {
    let rig = Rig::new("mn-handoff-fail-print", &[]);
    let cluster = &rig.cluster;
    std::fs::write(rig.log.join("hold"), b"").unwrap();
    std::fs::write(rig.log.join("launch-record.fail"), b"").unwrap();
    let (code, stdout, stderr) = std::thread::scope(|scope| {
        let run = scope.spawn(|| rig.handoff_pane(P2, &["--deadline", "600"], "60000"));
        wait_child_started(&rig);
        cluster.partition(0, 1);
        std::fs::remove_file(rig.log.join("hold")).unwrap();
        cluster.wait_until("the launch to fail on node 1", crate::WAIT, || {
            (cluster
                .scalar(1, "SELECT state FROM peer_requests", &[])
                .as_deref()
                == Some("failed"))
            .then_some(())
        });
        assert_eq!(rig.ledger()[0].0, "sent", "the sender heard nothing");
        // Replication must not link the row before the failed answer is polled.
        cluster.write_sql(
            0,
            "UPDATE handoff_obligations SET message='unlinked-'||message",
        );
        cluster.heal(0, 1);
        run.join().unwrap()
    });
    assert_ne!(code, 0, "{stdout}{stderr}");
    let staged = rig.task_rows(1);
    assert_eq!(staged.len(), 1, "{staged:?}");
    let (message, thread) = {
        let row = cluster.rows(
            1,
            &format!("SELECT id, thread_id FROM messages WHERE body='{TASK}'"),
        );
        (row[0][0].clone(), row[0][1].clone())
    };
    assert!(
        stderr.contains(&format!("; message {message} stands in thread {thread}")),
        "the CLI names the staged message: {stderr}"
    );
    assert!(!rig.record().exists(), "no harness was started");
    assert_eq!(rig.ledger()[0].0, "sent", "the obligation stays open");
}

// Kills: a launch refused by the target that still authors a fact somewhere, a refusal that
// leaves no ledger answer (or re-sends), and a CLI-side refusal that records an obligation.
#[test]
fn refusals_author_nothing() {
    let rig = Rig::new("mn-handoff-refuse", &[]);
    let cluster = &rig.cluster;
    let facts = || (cluster.kinds(0), cluster.kinds(1));
    let before = facts();

    // Refused by the CLI before sending: no row, no frame.
    let pane_args = rig.args(
        &rig.pane(P2).iter().map(String::as_str).collect::<Vec<_>>(),
        &[],
    );
    let mut no_summary = pane_args.clone();
    let at = no_summary.iter().position(|a| a == "--summary").unwrap();
    no_summary.drain(at..at + 2);
    let (code, _, stderr) = rig.run(&no_summary, "20000");
    assert_ne!(code, 0);
    assert!(stderr.contains("--summary is required"), "{stderr}");
    // A remote pane with --profile-dir: the guarded-pane refusal.
    let mut pane_profile = pane_args.clone();
    let at = pane_profile.iter().position(|a| a == "--").unwrap();
    pane_profile.splice(
        at..at,
        [
            "--profile-dir".to_owned(),
            rig.profile.display().to_string(),
        ],
    );
    let (code, _, stderr) = rig.run(&pane_profile, "20000");
    assert_ne!(code, 0);
    assert!(
        stderr.contains("guarded pane launches cannot change directories"),
        "{stderr}"
    );
    let tab = |rig: &Rig| {
        rig.args(
            &[
                "--new-tab",
                "x",
                "--space",
                &format!("w1@{}", rig.tag),
                "--cwd",
                "/tmp",
            ],
            &[],
        )
    };
    let relative = tab(&rig);
    let mut relative_dir = relative.clone();
    let at = relative_dir
        .iter()
        .position(|a| a == "--profile-dir")
        .unwrap();
    relative_dir[at + 1] = "relative/dir".into();
    let (code, _, stderr) = rig.run(&relative_dir, "20000");
    assert_ne!(code, 0);
    assert!(
        stderr.contains("must be an absolute path there"),
        "{stderr}"
    );
    assert_eq!(rig.ledger(), Vec::<(String, String)>::new());

    // Refused by the target in `decide`: one `failed` row holding the answer each, nothing run.
    let missing_dir = rig.profile.join("not-there");
    let mut bad_dir = relative.clone();
    let at = bad_dir.iter().position(|a| a == "--profile-dir").unwrap();
    bad_dir[at + 1] = missing_dir.display().to_string();
    let cases: Vec<(&str, Vec<String>, &str)> = vec![
        (
            "bad pane",
            {
                let target = rig.pane("nosuchpane");
                rig.args(&target.iter().map(String::as_str).collect::<Vec<_>>(), &[])
            },
            "no such pane here",
        ),
        ("bad profile dir", bad_dir, "profile"),
    ];
    for (name, args, needle) in cases {
        let (code, _, stderr) = rig.run(&args, "20000");
        assert_ne!(code, 0, "{name}");
        assert!(stderr.contains(needle), "{name}: {stderr}");
    }
    let ledger = rig.ledger();
    assert_eq!(ledger.len(), 2, "{ledger:?}");
    assert!(
        ledger.iter().all(|r| r.0 == "failed" && !r.1.is_empty()),
        "terminal rows holding the answer: {ledger:?}"
    );
    assert_eq!(calls(&rig.log).len(), 0, "no child ran for a refusal");
    assert_eq!(facts(), before, "no fact on any node");
}

/// Sends one handoff while node 1 is cut off (it stays `sent`), rewrites the frozen request
/// bytes with `tamper`, and heals: the target decides the rewritten request. Returns the ledger
/// row afterwards.
fn tampered_request(
    label: &str,
    tamper: impl FnOnce(&mut serde_json::Value),
) -> (Rig, (String, String)) {
    let rig = Rig::new(label, &[]);
    let cluster = &rig.cluster;
    let before = (cluster.kinds(0), cluster.kinds(1));
    cluster.partition(0, 1);
    // Unreachable: the request is signed and recorded, nothing is delivered.
    let (code, _, stderr) = rig.handoff_pane(P2, &[], "3000");
    assert_ne!(code, 0);
    assert!(
        stderr.contains("still pending") || stderr.contains("unreachable"),
        "{stderr}"
    );
    assert_eq!(rig.ledger().len(), 1, "the signed request is on the ledger");
    let bytes: Vec<u8> = {
        let conn = cluster.open_store(0).expect("node 0's store");
        conn.query_row("SELECT request_bytes FROM handoff_obligations", [], |r| {
            r.get(0)
        })
        .unwrap()
    };
    let mut request: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    tamper(&mut request);
    let hex: String = serde_json::to_vec(&request)
        .unwrap()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    cluster.write_sql(
        0,
        &format!("UPDATE handoff_obligations SET request_bytes=X'{hex}'"),
    );
    cluster.heal(0, 1);
    let row = cluster.wait_until("the target's refusal on the ledger", crate::WAIT, || {
        rig.ledger().into_iter().next().filter(|r| r.0 == "failed")
    });
    assert_eq!(
        (cluster.kinds(0), cluster.kinds(1)),
        before,
        "a refused request authors no fact on any node"
    );
    assert_eq!(calls(&rig.log).len(), 0, "no child ran");
    (rig, row)
}

// Kills: a request whose signature is not verified by the target (a forged sender block would
// be staged as a signed handoff).
#[test]
fn tampered_signature_is_refused() {
    let (_rig, row) = tampered_request("mn-handoff-badsig", |request| {
        let sig = request["plan"]["message"]["sig"]
            .as_str()
            .unwrap()
            .to_owned();
        let first = if sig.starts_with('0') { "1" } else { "0" };
        request["plan"]["message"]["sig"] = format!("{first}{}", &sig[1..]).into();
    });
    assert!(
        !row.1.is_empty(),
        "the refusal is the stored answer: {row:?}"
    );
}

// Kills: a harness the target does not register that is accepted and launched anyway.
#[test]
fn unregistered_harness_is_refused() {
    let (_rig, row) = tampered_request("mn-handoff-badkind", |request| {
        request["plan"]["kind"] = "no-such-harness".into();
    });
    assert!(
        !row.1.is_empty(),
        "the refusal is the stored answer: {row:?}"
    );
}

// Kills: a sender that sends a handoff to a target without `peer.handoff.v1` (the old node
// would launch a bare agent and drop the task), a missing upgrade error, and a refusal that
// leaves no ledger answer.
#[test]
fn withheld_cap_is_an_upgrade_error() {
    let rig = Rig::new(
        "mn-handoff-nocap",
        &[(1, "HT_TEST_WITHHOLD_CAPS", "peer.handoff.v1")],
    );
    let before = (rig.cluster.kinds(0), rig.cluster.kinds(1));
    let (code, stdout, stderr) = rig.handoff_pane(P2, &[], "20000");
    assert_ne!(code, 0, "{stdout}");
    assert!(
        stderr.contains("cannot take a handoff (needs peer.handoff.v1;"),
        "{stderr}"
    );
    assert!(stderr.contains("upgrade it to 3.0.0 or later"), "{stderr}");
    assert_eq!(
        calls(&rig.log).len(),
        0,
        "nothing reached node 1's launcher"
    );
    assert_eq!(
        rig.cluster
            .scalar(1, "SELECT count(*) FROM peer_requests", &[])
            .as_deref(),
        Some("0")
    );
    let ledger = rig.ledger();
    assert_eq!(ledger.len(), 1, "{ledger:?}");
    assert_eq!(ledger[0].0, "failed", "{ledger:?}");
    assert!(!ledger[0].1.is_empty(), "the NoCap answer is stored");
    assert_eq!((rig.cluster.kinds(0), rig.cluster.kinds(1)), before);
}

// Kills: a linked handoff whose receipt no node records overdue (the owner filter reading the
// authoring node instead of the deadline owner), an overdue recorded on the target, and a second
// warning after the handoff one.
#[test]
fn linked_then_overdue_on_the_sender_only() {
    let rig = Rig::new("mn-handoff-linked-overdue", &[]);
    let cluster = &rig.cluster;
    let (code, stdout, stderr) = rig.handoff_pane(P2, &["--deadline", "8"], "60000");
    assert_eq!(code, 0, "{stdout}{stderr}");
    let thread = field(&stdout, "thread").to_owned();
    // The order under test: the message links the ledger row before the deadline, and nobody ACKs.
    rig.wait_ledger("linked");
    assert_eq!(
        cluster
            .scalar(
                0,
                "SELECT state || '|' || (overdue_seq IS NULL) || '|' || (updated_at < deadline_at) \
                 FROM handoff_obligations",
                &[]
            )
            .as_deref(),
        Some("linked|1|1"),
        "the row must link before its deadline for this test to exercise the link-first order"
    );
    let overdue_in_thread = |node: usize, pattern: &str| {
        cluster
            .scalar(
                node,
                &format!(
                    "SELECT count(*) FROM messages WHERE thread_id=?1 AND event_key LIKE '{pattern}'"
                ),
                &[&thread],
            )
            .unwrap()
            .parse::<u32>()
            .unwrap()
    };
    cluster.wait_until(
        "the sender's receipt overdue in the pair thread",
        crate::WAIT,
        || (overdue_in_thread(0, "overdue:receipt:%") == 1).then_some(()),
    );
    cluster.wait_converged(crate::WAIT);
    let total = |node: usize| {
        cluster
            .scalar(
                node,
                "SELECT count(*) FROM messages WHERE event_key LIKE 'overdue:%'",
                &[],
            )
            .unwrap()
    };
    assert_eq!(total(0), "1", "exactly one overdue on the sender node");
    assert_eq!(overdue_in_thread(0, "overdue:handoff:%"), 0);
    assert_eq!(total(1), "0", "nothing on the target");
}

// Kills: an overdue that the sender's node never raises while the target is cut off (the sender
// is never told), a seat-addressed alert that is not shown in the inbox or `peer list`, an
// in-thread warning that fires twice or on the target, an alert that survives the real
// `Launched` answer or the link, and a `Launched` answer that never reaches the ledger after the
// partition heals (the re-send).
#[test]
fn overdue_while_partitioned() {
    let rig = Rig::new("mn-handoff-overdue", &[]);
    let cluster = &rig.cluster;
    let warnings = |node| {
        cluster
            .scalar(
                node,
                "SELECT count(*) FROM messages WHERE event_key LIKE 'overdue:%'",
                &[],
            )
            .unwrap()
            .parse::<u32>()
            .unwrap()
    };
    std::fs::write(rig.log.join("hold"), b"").unwrap();
    let out = std::thread::scope(|scope| {
        let run = scope.spawn(|| rig.handoff_pane(P2, &["--deadline", "3"], "12000"));
        wait_child_started(&rig);
        // Cut before the target answers.
        cluster.partition(0, 1);
        let alert = cluster.wait_until("the seat-addressed alert", crate::WAIT, || {
            let (_, inbox, _) = cluster.text_as(Some(&rig.a), 0, &["inbox"]);
            inbox.contains("overdue_handoff").then_some(inbox)
        });
        assert!(
            alert.contains(&format!("to {}", rig.tag))
                && alert.contains("not confirmed by that machine"),
            "{alert}"
        );
        let (_, peers, _) = cluster.text_as(None, 0, &["peer", "list"]);
        assert!(
            peers.contains("overdue: not confirmed by that machine"),
            "{peers}"
        );
        // No in-thread warning while the seat is unknown.
        assert_eq!(
            cluster
                .scalar(
                    0,
                    "SELECT count(*) FROM messages WHERE event_key LIKE 'overdue:handoff:%'",
                    &[]
                )
                .as_deref(),
            Some("0")
        );
        run.join().unwrap()
    });
    assert_ne!(out.0, 0, "the poll gave up while cut off: {out:?}");
    // Heal and release the target's launch: its real `Launched` answer (fetched by the re-send
    // on link-up) and the replicated message reach node 0, in either order.
    cluster.heal(0, 1);
    std::fs::remove_file(rig.log.join("hold")).unwrap();
    cluster.wait_until(
        "exactly one overdue warning on the sender",
        crate::WAIT,
        || (warnings(0) == 1).then_some(()),
    );
    rig.wait_ledger("linked");
    assert_eq!(
        cluster
            .scalar(0, "SELECT seat IS NOT NULL FROM handoff_obligations", &[])
            .as_deref(),
        Some("1"),
        "the ledger learned the seat from a real answer or the link"
    );
    // Linking neither repeats the warning nor puts one on the target.
    cluster.wait_converged(crate::WAIT);
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(warnings(1), 0, "no overdue on the target");
    assert_eq!(warnings(0), 1, "exactly one overdue warning on the sender");
    // The alert was shown by the inbox read above and is not shown again.
    let (_, inbox, _) = cluster.text_as(Some(&rig.a), 0, &["inbox"]);
    assert!(!inbox.contains("overdue_handoff"), "{inbox}");
}

// Kills: a handoff `message.sent` whose sender block is signed by the wrong node (a forged
// handoff) that other nodes apply: it must be `failed` on every node that is not its origin,
// and listed by `sync unapplied`.
#[test]
fn tampered_block_three_nodes() {
    use herdr_threads::node::{NodeIdentity, tag_for};
    use herdr_threads::protocol::ids::{
        MessageId, generate_public_id, prefix, send_message_id_for_preparation,
    };
    use herdr_threads::sync::facts::message::MessageSent;
    use herdr_threads::sync::handoff_block::SenderBlock;
    use herdr_threads::test_support::multinode::default_seed;

    let mut cluster = Cluster::start(
        "mn-handoff-tamper",
        crate::BIN,
        3,
        ClusterOptions {
            links: vec![(0, 1), (2, 1)],
            ..Default::default()
        },
    );
    cluster.add_host(0, &[P1]);
    cluster.add_host(1, &[P1]);
    let a = cluster.seat(0, P1);
    let r = cluster.seat(1, P1);
    // The send below runs on node 0, so node 0 must know the recipient seat first.
    support::wait_seats(&cluster, 0, 2);
    support::wait_seats(&cluster, 1, 2);
    support::wait_seats(&cluster, 2, 2);
    // The ordinary send opens the pair thread everywhere.
    support::send(&cluster, &a, &["--seat", &r.id, "--body", "hello"]);
    cluster.wait_converged(crate::WAIT);
    let message = MessageId::new(
        send_message_id_for_preparation(&generate_public_id(
            prefix::SEND_PREPARATION,
            &tag_for(cluster.node_uuid(1), 0),
        ))
        .unwrap(),
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let mut block = SenderBlock {
        v: 1,
        request: uuid::Uuid::from_u128(11),
        sender_node: Some(cluster.node_uuid(0)),
        author: Some(SeatId::new(a.id.clone())),
        author_role: None,
        message: Some(message.clone()),
        body: "take over".into(),
        priority: 0,
        require_ack: true,
        requested_at: now,
        ack_after_ms: Some(300_000),
        sig: String::new(),
    };
    // Signed by node 2's key although the block names node 0 as the sender.
    block
        .sign(&NodeIdentity::from_seed(default_seed(2)))
        .unwrap();
    let payload = block
        .expected_payload(&SeatId::new(r.id.clone()), now)
        .unwrap();
    let (counter, _) = cluster.author_builtin::<MessageSent>(1, &payload);
    cluster.wait_converged(crate::WAIT);
    for node in [0, 2] {
        let entry = cluster
            .journal_entry(node, 1, counter)
            .unwrap_or_else(|| panic!("node {node} holds the entry"));
        assert_eq!(entry.kind, "message.sent");
        assert_eq!(entry.apply_state, "failed", "node {node}");
        assert_eq!(
            cluster
                .scalar(
                    node,
                    &format!(
                        "SELECT count(*) FROM messages WHERE id='{}'",
                        message.as_str()
                    ),
                    &[]
                )
                .as_deref(),
            Some("0"),
            "node {node} materialised nothing"
        );
        let out = cluster.cli(node, &["sync", "unapplied"]);
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(text.contains("message.sent"), "node {node}: {text}");
    }
}

// Kills: a handoff `message.sent` that a pre-3.0 node (SeatHome-only handler, daemon-level
// stand-in `HT_TEST_LEGACY_MESSAGE_SENT`) applies or drops silently instead of marking it
// `failed` and listing it in `sync unapplied`, and a replay after the upgrade that does not
// apply it. The two upgraded nodes apply it at once.
#[test]
fn older_third_node_fails_then_replays_after_upgrade() {
    use herdr_threads::node::{NodeIdentity, tag_for};
    use herdr_threads::protocol::ids::{
        MessageId, generate_public_id, prefix, send_message_id_for_preparation,
    };
    use herdr_threads::sync::facts::message::MessageSent;
    use herdr_threads::sync::handoff_block::SenderBlock;
    use herdr_threads::test_support::multinode::default_seed;

    let flag = herdr_threads::test_support::scratch::temp_path(format!(
        "ht-legacy-sent-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::write(&flag, b"").unwrap();
    let mut cluster = Cluster::start(
        "mn-handoff-older",
        crate::BIN,
        3,
        ClusterOptions {
            links: vec![(0, 1), (2, 1)],
            env: vec![(
                2,
                "HT_TEST_LEGACY_MESSAGE_SENT".to_owned(),
                flag.display().to_string(),
            )],
            ..Default::default()
        },
    );
    cluster.add_host(0, &[P1]);
    cluster.add_host(1, &[P1]);
    let a = cluster.seat(0, P1);
    let r = cluster.seat(1, P1);
    // The send below runs on node 0, so node 0 must know the recipient seat first.
    support::wait_seats(&cluster, 0, 2);
    support::wait_seats(&cluster, 1, 2);
    support::wait_seats(&cluster, 2, 2);
    support::send(&cluster, &a, &["--seat", &r.id, "--body", "hello"]);
    cluster.wait_converged(crate::WAIT);
    let message = MessageId::new(
        send_message_id_for_preparation(&generate_public_id(
            prefix::SEND_PREPARATION,
            &tag_for(cluster.node_uuid(1), 0),
        ))
        .unwrap(),
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let mut block = SenderBlock {
        v: 1,
        request: uuid::Uuid::from_u128(12),
        sender_node: Some(cluster.node_uuid(0)),
        author: Some(SeatId::new(a.id.clone())),
        author_role: None,
        message: Some(message.clone()),
        body: "take over".into(),
        priority: 0,
        require_ack: true,
        requested_at: now,
        ack_after_ms: Some(300_000),
        sig: String::new(),
    };
    // A valid handoff: signed by the sender node's own key.
    block
        .sign(&NodeIdentity::from_seed(default_seed(0)))
        .unwrap();
    let payload = block
        .expected_payload(&SeatId::new(r.id.clone()), now)
        .unwrap();
    let (counter, _) = cluster.author_builtin::<MessageSent>(1, &payload);
    let materialised = |cluster: &Cluster, node| {
        cluster
            .scalar(
                node,
                &format!(
                    "SELECT count(*) FROM messages WHERE id='{}'",
                    message.as_str()
                ),
                &[],
            )
            .as_deref()
            == Some("1")
    };
    for node in [0, 1] {
        cluster.wait_until("the upgraded node to apply it", crate::WAIT, || {
            (cluster
                .journal_entry(node, 1, counter)
                .map(|e| e.apply_state)
                == Some("applied".to_owned())
                && materialised(&cluster, node))
            .then_some(())
        });
    }
    let failed = cluster.wait_until("the older node to mark it failed", crate::WAIT, || {
        cluster
            .journal_entry(2, 1, counter)
            .filter(|e| e.apply_state == "failed")
    });
    assert_eq!(failed.kind, "message.sent");
    assert!(!materialised(&cluster, 2), "the older node applied nothing");
    let out = cluster.cli(2, &["sync", "unapplied"]);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("message.sent"), "{text}");

    // Upgrade: the shipped handler on the same store, then replay.
    std::fs::remove_file(&flag).unwrap();
    cluster.stop_node(2);
    cluster.start_node(2);
    let out = cluster.human(2, &["sync", "unapplied", "replay"]);
    assert!(out.status.success(), "{out:?}");
    cluster.wait_until("the replay to apply it", crate::WAIT, || {
        (cluster.journal_entry(2, 1, counter).map(|e| e.apply_state) == Some("applied".to_owned())
            && materialised(&cluster, 2))
        .then_some(())
    });
}
