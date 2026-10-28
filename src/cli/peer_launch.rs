//! `internal peer-tab-create`: the tab a peer-requested launch opens
//! (remote launch, TRUST-POLICY N15).
//!
//! Run only by this node's daemon, as a child with a scrubbed environment, for a
//! `launch_request` it recorded in `peer_requests`; the launch itself is then an
//! ordinary `launch --pane` child. It creates one tab in one workspace of the
//! `--host-endpoint` Herdr server (witnessed by a `ping` on the same server,
//! never focused) and prints `{"pane":..,"tab":..}`. It reads and writes no seat,
//! binding or journal and claims no caller.

use std::{io::Write, path::Path, sync::Arc, time::Duration};

use super::RunError;
use crate::{
    ports::{CreateTabOutcome, CreateTabPort, CreateTabRequest, HostCallContext},
    protocol::{
        ids::{HostCallId, HostTargetId},
        results::{ApiError, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
};

/// Budget of the witness ping and the creation together (ms).
const BUDGET_MS: u64 = 30_000;

pub fn create_tab<W: Write>(
    endpoint: &Path,
    workspace: &str,
    label: &str,
    cwd: &Path,
    clock: Arc<dyn Clock>,
    out: &mut W,
) -> Result<(), RunError> {
    let budget = CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0.saturating_add(BUDGET_MS)),
        cancellation: Cancellation::default(),
    };
    let witness = crate::host::request_witnessed(
        endpoint,
        &uuid::Uuid::new_v4().to_string(),
        "ping",
        serde_json::json!({}),
        clock.as_ref(),
        &budget,
        Duration::from_secs(5),
    )?
    .witness;
    let host = crate::host::native::NativeCli::new(endpoint.to_path_buf(), Arc::clone(&clock));
    let request = CreateTabRequest {
        correlation: HostCallId::new(uuid::Uuid::new_v4().to_string()),
        workspace: HostTargetId::parse(workspace.to_owned())
            .map_err(|detail| ApiError::new(ErrorCode::InvalidRequest, detail))?,
        cwd: cwd.to_path_buf(),
        label: label.to_owned(),
        focus: false,
        env: Default::default(),
        directory_context: None,
        expected_witness: witness,
    };
    let context = HostCallContext {
        budget,
        expected_boot: None,
        expected_epoch: None,
    };
    match host.create_tab(&request, &context) {
        CreateTabOutcome::Created(created) => {
            let line = serde_json::json!({
                "pane": created.root_pane.as_str(),
                "tab": created.tab.as_str(),
            });
            writeln!(out, "{line}")?;
            Ok(())
        }
        CreateTabOutcome::NotSubmitted(error) | CreateTabOutcome::OutcomeUnknown(error) => {
            Err(error.into())
        }
    }
}

/// A launch on another machine, and with `then` the handoff into its seat
/// (remote launch, TRUST-POLICY N15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteLaunch {
    pub machine: String,
    pub plan: crate::sync::frame::LaunchPlan,
    /// The handoff to run once the seat exists here: `--existing --seat`.
    pub then: Option<Box<super::topology_handoff::Request>>,
}

/// How long the CLI keeps asking how a remote launch stands.
pub const REMOTE_LAUNCH_TIMEOUT: Duration = Duration::from_secs(120);
/// How often it asks.
const POLL: Duration = Duration::from_secs(2);

/// [`REMOTE_LAUNCH_TIMEOUT`]; test-support builds read
/// `HT_TEST_REMOTE_LAUNCH_TIMEOUT_MS` (multinode tests of a rerun).
fn launch_timeout() -> Duration {
    #[cfg(feature = "test-support")]
    if let Some(ms) = std::env::var("HT_TEST_REMOTE_LAUNCH_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        return Duration::from_millis(ms);
    }
    REMOTE_LAUNCH_TIMEOUT
}

/// The request of one remote launch as this node keeps it, in
/// `node_dir/remote-launch/<key>.id`: its id, and once the follow-on handoff
/// recorded its intent, that intent's recovery reference (`journaled`), so a
/// rerun resumes through it and never hands the task off a second time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestRecord {
    pub id: uuid::Uuid,
    pub journaled: Option<String>,
}

impl RequestRecord {
    fn parse(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        let id = lines.next()?.trim().parse().ok()?;
        let journaled = lines
            .find_map(|line| line.strip_prefix("journaled "))
            .map(|r| r.trim().to_owned());
        Some(Self { id, journaled })
    }

    fn text(&self) -> String {
        match &self.journaled {
            Some(reference) => format!("{}\njournaled {reference}\n", self.id),
            None => format!("{}\n", self.id),
        }
    }
}

/// Holds the per-request lock: every read and replacement of one request's
/// file happens under it, so concurrent runs of one command agree.
struct RecordLock {
    /// Holds the lock while open; closing it releases the lock.
    _file: std::fs::File,
}

impl RecordLock {
    fn take(file: &Path) -> std::io::Result<Self> {
        use std::os::unix::io::AsRawFd;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(file.with_extension("lock"))?;
        // SAFETY: flock on a descriptor this function owns.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self { _file: lock })
    }
}

/// Writes `record` to `file` atomically (temporary file, sync, rename).
fn write_record(file: &Path, record: &RequestRecord) -> std::io::Result<()> {
    use std::io::Write as _;
    let tmp = file.with_extension(format!("{}.tmp", std::process::id()));
    let mut out = std::fs::File::create(&tmp)?;
    out.write_all(record.text().as_bytes())?;
    out.sync_all()?;
    std::fs::rename(&tmp, file)
}

/// The key of this launch's request: the whole frozen intent (the resolved
/// node, the requesting seat, the plan and the handoff that follows it), so a
/// rerun of the same command finds its request and any other command (another
/// task, another seat) never does.
fn request_key(node_uuid: &str, requester_seat: Option<&str>, remote: &RemoteLaunch) -> String {
    use sha2::Digest;
    let mut hash = sha2::Sha256::new();
    hash.update(b"herdr-threads remote launch v3\0");
    hash.update(node_uuid.as_bytes());
    hash.update(b"\0");
    hash.update(requester_seat.unwrap_or_default().as_bytes());
    hash.update(b"\0");
    hash.update(serde_json::to_vec(&remote.plan).unwrap_or_default());
    hash.update(b"\0");
    hash.update(serde_json::to_vec(&remote.then).unwrap_or_default());
    hash.finalize()
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// This launch's request: the one a run of the same command recorded (kept
/// until the launch settles, so a rerun resumes it and never launches twice),
/// else a new one. Under the request's lock, so concurrent runs agree; an
/// unreadable file a dead run left behind is replaced.
fn request_id(
    node_dir: &Path,
    node_uuid: &str,
    requester_seat: Option<&str>,
    remote: &RemoteLaunch,
) -> std::io::Result<(RequestRecord, std::path::PathBuf)> {
    let key = request_key(node_uuid, requester_seat, remote);
    let dir = node_dir.join("remote-launch");
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(format!("{key}.id"));
    let _lock = RecordLock::take(&file)?;
    if let Some(record) = std::fs::read_to_string(&file)
        .ok()
        .and_then(|text| RequestRecord::parse(&text))
    {
        return Ok((record, file));
    }
    let record = RequestRecord {
        id: uuid::Uuid::new_v4(),
        journaled: None,
    };
    write_record(&file, &record).map_err(|error| {
        std::io::Error::other(format!(
            "remote launch request file {}: {error}",
            file.display()
        ))
    })?;
    Ok((record, file))
}

/// The refusal for a machine that names no single other node.
fn machine_refusal(machine: &str, reply: crate::protocol::sync_admin::LaunchReply) -> RunError {
    use crate::protocol::sync_admin::LaunchReply;
    let refuse =
        |code: ErrorCode, detail: String| -> RunError { ApiError::new(code, detail).into() };
    match reply {
        LaunchReply::SelfNode => refuse(
            ErrorCode::InvalidRequest,
            format!(
                "{} is this machine: launch without @MACHINE",
                one_line(machine)
            ),
        ),
        LaunchReply::UnknownMachine { known } => {
            let names: Vec<String> = known.iter().take(8).map(label_of).collect();
            refuse(
                ErrorCode::NotFound,
                format!(
                    "no machine {}; known: {}; for a local pane whose label has an @, pass its pane ID",
                    one_line(machine),
                    if names.is_empty() {
                        "none".to_owned()
                    } else {
                        names.join(", ")
                    }
                ),
            )
        }
        LaunchReply::AmbiguousMachine { matches } => {
            let lines: Vec<String> = matches
                .iter()
                .map(|row| format!("  {} {}", row.uuid, label_of(row)))
                .collect();
            refuse(
                ErrorCode::Conflict,
                format!(
                    "ambiguous machine {}: {} matches\n{}\npass @<uuid>",
                    one_line(machine),
                    matches.len(),
                    lines.join("\n")
                ),
            )
        }
        LaunchReply::Remote { .. } | LaunchReply::Unknown => refuse(
            ErrorCode::InvalidRequest,
            "the daemon answered in an unknown form".into(),
        ),
    }
}

fn label_of(node: &crate::protocol::sync_admin::MachineRow) -> String {
    match &node.host {
        Some(host) => format!(
            "{} {}",
            node.tag,
            crate::view::escape::escape_for_terminal(
                host,
                crate::view::escape::Context::SingleLine
            )
        ),
        None => node.tag.clone(),
    }
}

fn one_line(text: &str) -> String {
    crate::view::escape::escape_for_terminal(text, crate::view::escape::Context::SingleLine)
        .into_owned()
}

/// Whether a remote handoff's follow-on result ends its launch's request
/// before the handoff journaled anything: a success, or a refusal asking
/// again cannot change (the launched seat cannot take it). A transient
/// failure keeps the request, so a rerun resumes at delivery and never
/// launches a second agent. (Once the handoff journaled its intent, the
/// request keeps that intent's reference instead, and a rerun resumes
/// through `retry`.)
pub fn handoff_settles(result: &Result<(), RunError>) -> bool {
    match result {
        Ok(()) => true,
        Err(RunError::Api(error)) => matches!(
            error.code,
            ErrorCode::InvalidRequest | ErrorCode::NotFound | ErrorCode::Conflict
        ),
        Err(_) => false,
    }
}

/// A launch that started on another machine. Its request id stays recorded
/// until [`Self::settle`], so a rerun before it (a failed follow-on handoff)
/// asks for the same launch's outcome instead of launching again.
#[derive(Debug)]
pub struct Launched {
    pub seat: String,
    pub pane: String,
    id: uuid::Uuid,
    file: std::path::PathBuf,
}

impl Launched {
    /// The launch's work is done here: a new launch of the same plan gets a
    /// new request id.
    pub fn settle(&self) {
        if let Ok(_lock) = RecordLock::take(&self.file) {
            let _ = std::fs::remove_file(&self.file);
        }
    }

    /// The follow-on handoff recorded its intent `reference`: kept with the
    /// request before delivery starts, so a rerun resumes through it.
    pub fn record_journaled(&self, reference: &str) -> std::io::Result<()> {
        let _lock = RecordLock::take(&self.file)?;
        write_record(
            &self.file,
            &RequestRecord {
                id: self.id,
                journaled: Some(reference.to_owned()),
            },
        )
    }
}

/// Runs a remote launch to its outcome: the launched seat (now held here as
/// that machine's) and its pane, or a refusal.
pub fn run_remote<C: crate::ports::LocalClient + ?Sized, W: Write>(
    remote: &RemoteLaunch,
    requester_seat: Option<&str>,
    node_dir: &Path,
    client: &C,
    clock: &dyn Clock,
    stderr: &mut W,
) -> Result<Launched, RunError> {
    use crate::protocol::sync_admin::{LaunchReply, RemoteLaunchAnswer, SyncCommand, SyncReply};
    use crate::sync::frame::{LaunchRequest, LaunchState};
    let budget = || CallBudget {
        deadline: MonoInstant(
            clock
                .monotonic_now()
                .0
                .saturating_add(super::seat_resolution::REMOTE_ADDRESS_BUDGET_MS),
        ),
        cancellation: Cancellation::default(),
    };
    let node = match client.call(
        crate::protocol::commands::Command::Sync(SyncCommand::ResolveMachine {
            machine: remote.machine.clone(),
        }),
        &budget(),
    )? {
        crate::protocol::results::CommandResult::Sync(SyncReply::Launch(LaunchReply::Remote {
            node,
            ..
        })) => node,
        crate::protocol::results::CommandResult::Sync(SyncReply::Launch(other)) => {
            return Err(machine_refusal(&remote.machine, other));
        }
        _ => {
            return Err(ApiError::new(
                ErrorCode::InvalidRequest,
                "unexpected machine resolution result",
            )
            .into());
        }
    };
    let (record, file) = request_id(node_dir, &node.uuid, requester_seat, remote)?;
    // The follow-on handoff of this very command already recorded its intent:
    // it resumes through that intent, never through a second handoff.
    let record = match &record.journaled {
        Some(reference) => {
            let journal = super::journal::Journal::read_only(node_dir.join("intents"))?;
            match journal.resolve_recovery_ref(reference) {
                Ok(_) => {
                    return Err(ApiError::new(
                        ErrorCode::Conflict,
                        format!(
                            "this handoff is already journaled as {reference}; finish it with \
                             `herdr-threads retry {reference}`"
                        ),
                    )
                    .into());
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    // That handoff completed: this run is a new command.
                    if let Ok(_lock) = RecordLock::take(&file) {
                        let _ = std::fs::remove_file(&file);
                    }
                    request_id(node_dir, &node.uuid, requester_seat, remote)?.0
                }
                Err(error) => return Err(error.into()),
            }
        }
        None => record,
    };
    let id = record.id;
    let request = LaunchRequest {
        id,
        requester_seat: requester_seat.unwrap_or_default().to_owned(),
        plan: remote.plan.clone(),
    };
    let settle = || {
        if let Ok(_lock) = RecordLock::take(&file) {
            let _ = std::fs::remove_file(&file);
        }
    };
    let refuse =
        |code: ErrorCode, detail: String| -> RunError { ApiError::new(code, detail).into() };
    let started = std::time::Instant::now();
    let mut announced = false;
    loop {
        let reply = client.call(
            // By the node's full uuid: a node-table change mid-poll never retargets it.
            crate::protocol::commands::Command::Sync(SyncCommand::RemoteLaunch {
                machine: node.uuid.clone(),
                request: Box::new(request.clone()),
            }),
            &budget(),
        )?;
        let crate::protocol::results::CommandResult::Sync(SyncReply::Launch(reply)) = reply else {
            return Err(refuse(
                ErrorCode::InvalidRequest,
                "unexpected remote launch result".into(),
            ));
        };
        let resolved = &node.uuid;
        let (node, answer) = match reply {
            LaunchReply::Remote { node, answer } if &node.uuid == resolved => (node, answer),
            // Another node, or none: the request stays, nothing is settled.
            _ => {
                return Err(refuse(
                    ErrorCode::HostUnavailable,
                    format!(
                        "machine {resolved} is no longer the node this launch was asked of; \
                         request {} kept, rerun to resume",
                        &id.to_string()[..8]
                    ),
                ));
            }
        };
        let label = label_of(&node);
        let pending = match answer {
            RemoteLaunchAnswer::Answered { state } => match state {
                LaunchState::Launched { seat, pane } => {
                    writeln!(
                        stderr,
                        "launched: {} on {}@{} (request {})",
                        one_line(&seat),
                        one_line(&pane),
                        node.tag,
                        &id.to_string()[..8]
                    )?;
                    return Ok(Launched {
                        seat,
                        pane,
                        id,
                        file: file.clone(),
                    });
                }
                LaunchState::Refused { detail } => {
                    settle();
                    return Err(refuse(
                        ErrorCode::InvalidRequest,
                        format!("{label} refused the launch: {}", one_line(&detail)),
                    ));
                }
                LaunchState::Failed {
                    detail,
                    pane,
                    unconfirmed,
                } => {
                    // An unconfirmed start keeps the id: a rerun reports the same, never a
                    // second agent in a pane that may already run one.
                    if !unconfirmed {
                        settle();
                    }
                    let left = pane.map_or_else(String::new, |p| {
                        format!("; it left tab pane {} there", one_line(&p))
                    });
                    // An unconfirmed start keeps answering for this exact command
                    // (N15); any change to its task or plan is a new request.
                    let escape = if unconfirmed {
                        "; rerunning this exact command reports the same, change its task or plan \
                         to launch anew"
                    } else {
                        ""
                    };
                    return Err(refuse(
                        ErrorCode::HostUnavailable,
                        format!(
                            "launch on {label} failed: {}{left}{escape}",
                            one_line(&detail)
                        ),
                    ));
                }
                LaunchState::Running { .. } => "running".to_owned(),
                LaunchState::Unavailable { reason } => one_line(&reason),
                LaunchState::Unknown => "unknown answer".to_owned(),
            },
            RemoteLaunchAnswer::NoCap { version } => {
                settle();
                return Err(refuse(
                    ErrorCode::Unsupported,
                    format!(
                        "machine {label} cannot launch for other machines (needs peer.launch.v1; version {})",
                        version.as_deref().unwrap_or("unknown")
                    ),
                ));
            }
            RemoteLaunchAnswer::Unreachable { .. } => "unreachable".to_owned(),
            RemoteLaunchAnswer::Timeout { secs } => format!("no answer in {secs}s"),
            RemoteLaunchAnswer::NotReplicated { .. } => "launched; waiting for its seat".to_owned(),
            RemoteLaunchAnswer::Unknown => "unknown answer".to_owned(),
        };
        if !announced {
            writeln!(
                stderr,
                "launching on {label} (request {})",
                &id.to_string()[..8]
            )?;
            announced = true;
        }
        if started.elapsed() >= launch_timeout() {
            return Err(refuse(
                ErrorCode::HostUnavailable,
                format!(
                    "launch on {label} still pending ({pending}): request {}; rerun to resume",
                    &id.to_string()[..8]
                ),
            ));
        }
        std::thread::sleep(POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(task: &str) -> RemoteLaunch {
        RemoteLaunch {
            machine: "laptop".into(),
            plan: crate::sync::frame::LaunchPlan {
                target: crate::sync::frame::LaunchTarget::Pane {
                    space: None,
                    tab: None,
                    pane: "w1:p2".into(),
                },
                kind: "codex".into(),
                argv: Vec::new(),
                cwd: None,
            },
            then: Some(Box::new(super::super::topology_handoff::Request {
                policy: None,
                automatic: true,
                new_tab: None,
                existing: true,
                selector: Default::default(),
                seat: None,
                cwd: None,
                profile_dir: None,
                thread: None,
                thread_name: None,
                topic: None,
                goal: None,
                body: task.into(),
                kind: None,
                binary: None,
                name: None,
                argv: Vec::new(),
            })),
        }
    }

    // Kills: one request id for two different handoffs (task B landing in the
    // agent launched for task A), for two requesting seats, or for two nodes;
    // a different id on a rerun of the same command; and two concurrent runs
    // of one command each minting their own id (two launches).
    #[test]
    fn request_ids_follow_the_whole_frozen_intent() {
        let dir = crate::test_support::scratch::temp_path(format!(
            "ht-remote-launch-id-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let id = |node: &str, seat: Option<&str>, task: &str| {
            request_id(&dir, node, seat, &remote(task)).unwrap().0.id
        };
        let a = id("n1", Some("s1"), "task A");
        assert_eq!(a, id("n1", Some("s1"), "task A"), "a rerun reuses it");
        assert_ne!(a, id("n1", Some("s1"), "task B"), "another task");
        assert_ne!(a, id("n1", Some("s2"), "task A"), "another seat");
        assert_ne!(a, id("n2", Some("s1"), "task A"), "another node");
        // Concurrent first runs agree on one id.
        let ids: Vec<uuid::Uuid> = std::thread::scope(|scope| {
            let runs: Vec<_> = (0..8)
                .map(|_| scope.spawn(|| id("n3", Some("s1"), "race")))
                .collect();
            runs.into_iter().map(|r| r.join().unwrap()).collect()
        });
        assert!(ids.iter().all(|i| *i == ids[0]), "{ids:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Kills: an id file a dead run left empty blocking every later run, two
    // runs that find it at once replacing it with different ids, and the
    // follow-on handoff's intent reference not surviving with the request.
    #[test]
    fn the_request_record_is_replaced_once_and_keeps_the_journaled_intent() {
        let dir = crate::test_support::scratch::temp_path(format!(
            "ht-remote-launch-empty-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let (first, file) = request_id(&dir, "n1", Some("s1"), &remote("task")).unwrap();
        std::fs::write(&file, b"").unwrap();
        let ids: Vec<uuid::Uuid> = std::thread::scope(|threads| {
            let runs: Vec<_> = (0..8)
                .map(|_| {
                    threads.spawn(|| {
                        request_id(&dir, "n1", Some("s1"), &remote("task"))
                            .unwrap()
                            .0
                            .id
                    })
                })
                .collect();
            runs.into_iter().map(|r| r.join().unwrap()).collect()
        });
        assert!(ids.iter().all(|i| *i == ids[0]), "{ids:?}");
        assert_ne!(first.id, ids[0], "a fresh id replaced the empty file");
        let launched = Launched {
            seat: "s".into(),
            pane: "p".into(),
            id: ids[0],
            file: file.clone(),
        };
        launched.record_journaled("local:7").unwrap();
        let (record, _) = request_id(&dir, "n1", Some("s1"), &remote("task")).unwrap();
        assert_eq!(record.id, ids[0]);
        assert_eq!(record.journaled.as_deref(), Some("local:7"));
        launched.settle();
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Kills: a remote handoff that failed for a passing reason settling its
    // launch (a rerun would start a second agent), and one refused for good
    // keeping it (every rerun stuck on the old seat).
    #[test]
    fn only_success_or_a_final_refusal_settles_the_launch() {
        let api = |code| Err(RunError::Api(ApiError::new(code, "x")));
        assert!(handoff_settles(&Ok(())));
        assert!(handoff_settles(&api(ErrorCode::InvalidRequest)));
        assert!(handoff_settles(&api(ErrorCode::NotFound)));
        for passing in [
            ErrorCode::HostUnavailable,
            ErrorCode::UnknownOutcome,
            ErrorCode::StoreBusy,
        ] {
            let shown = format!("{passing:?}");
            assert!(!handoff_settles(&api(passing)), "{shown}");
        }
        assert!(!handoff_settles(&Err(RunError::Io(std::io::Error::other(
            "lost"
        )))));
    }
}
