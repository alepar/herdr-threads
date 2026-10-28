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

/// Holds the `remote-launch/` directory's one lock: every read, check and
/// replacement of a request file happens in one hold of it, so concurrent
/// runs of one command agree (and no per-request lock file accumulates).
struct RecordLock {
    /// Holds the lock while open; closing it releases the lock.
    _file: std::fs::File,
}

impl RecordLock {
    /// The lock of the directory holding request `file`.
    fn take(file: &Path) -> std::io::Result<Self> {
        use std::os::unix::io::AsRawFd;
        let dir = file
            .parent()
            .ok_or_else(|| std::io::Error::other("request file has no directory"))?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(".lock"))?;
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
    let written = (|| {
        let mut out = std::fs::File::create(&tmp)?;
        out.write_all(record.text().as_bytes())?;
        out.sync_all()?;
        std::fs::rename(&tmp, file)?;
        // The rename itself survives an OS crash, like the journal it pairs with.
        if let Some(dir) = file.parent() {
            std::fs::File::open(dir)?.sync_all()?;
        }
        Ok(())
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Removes request `file`, durably (the directory is synced).
fn remove_record(file: &Path) -> std::io::Result<()> {
    std::fs::remove_file(file)?;
    if let Some(dir) = file.parent() {
        std::fs::File::open(dir)?.sync_all()?;
    }
    Ok(())
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
/// else a new one. In one hold of the directory lock, so concurrent runs
/// agree; an unreadable file a dead run left behind is replaced.
///
/// A record whose follow-on handoff already journaled its intent: while that
/// intent is pending (`pending(ref)`), the command must resume through it
/// (`Err(Conflict)`, naming `retry`); once it completed, this run is a new
/// command and gets a new request.
fn request_id(
    node_dir: &Path,
    node_uuid: &str,
    requester_seat: Option<&str>,
    remote: &RemoteLaunch,
    pending: impl Fn(&str) -> std::io::Result<bool>,
) -> Result<(RequestRecord, std::path::PathBuf), RunError> {
    let key = request_key(node_uuid, requester_seat, remote);
    let dir = node_dir.join("remote-launch");
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(format!("{key}.id"));
    let _lock = RecordLock::take(&file)?;
    if let Some(record) = std::fs::read_to_string(&file)
        .ok()
        .and_then(|text| RequestRecord::parse(&text))
    {
        match &record.journaled {
            None => return Ok((record, file)),
            Some(reference) if pending(reference)? => {
                return Err(ApiError::new(
                    ErrorCode::Conflict,
                    format!(
                        "this handoff is already journaled as {reference}; finish it with \
                         `herdr-threads retry {reference}`"
                    ),
                )
                .into());
            }
            // That handoff completed: this run is a new command.
            Some(_) => {}
        }
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

/// Whether intent `reference` of the journal in `node_dir` is still pending.
fn intent_pending(node_dir: &Path, reference: &str) -> std::io::Result<bool> {
    let journal = super::journal::Journal::read_only(node_dir.join("intents"))?;
    match journal.resolve_recovery_ref(reference) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
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

/// What a remote handoff's outcome does to its launch's request.
#[derive(Debug, PartialEq, Eq)]
pub enum AfterHandoff {
    /// Done: a new run of the command is a new launch.
    Settle,
    /// Journaled, then failed: the request keeps the intent's reference, and a
    /// rerun is told to finish it with `retry`.
    KeepJournaled(String),
    /// Refused for good before journaling: settled, a rerun launches anew.
    SettleRefused,
    /// A transient failure before journaling: kept, a rerun resumes delivery.
    Keep,
}

/// The rule for a remote handoff's `result`, given the intent it journaled
/// on this thread (`journaled`), if any.
pub fn after_handoff(result: &Result<(), RunError>, journaled: Option<&str>) -> AfterHandoff {
    match (result, journaled) {
        (Ok(()), _) => AfterHandoff::Settle,
        (Err(_), Some(reference)) => AfterHandoff::KeepJournaled(reference.to_owned()),
        (Err(_), None) if handoff_settles(result) => AfterHandoff::SettleRefused,
        (Err(_), None) => AfterHandoff::Keep,
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
        if let Ok(_lock) = RecordLock::take(&self.file)
            && self.still_mine()
        {
            let _ = remove_record(&self.file);
        }
    }

    /// Whether the request file still names this launch (a later run of the
    /// same command may have replaced it).
    fn still_mine(&self) -> bool {
        std::fs::read_to_string(&self.file)
            .ok()
            .and_then(|text| RequestRecord::parse(&text))
            .is_some_and(|record| record.id == self.id)
    }

    /// The follow-on handoff recorded its intent `reference`: kept with the
    /// request before delivery starts, so a rerun resumes through it.
    pub fn record_journaled(&self, reference: &str) -> std::io::Result<()> {
        let _lock = RecordLock::take(&self.file)?;
        if !self.still_mine() {
            return Err(std::io::Error::other(
                "the request file names another run's launch",
            ));
        }
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
    let (record, file) = request_id(node_dir, &node.uuid, requester_seat, remote, |reference| {
        intent_pending(node_dir, reference)
    })?;
    let id = record.id;
    let request = LaunchRequest {
        id,
        requester_seat: requester_seat.unwrap_or_default().to_owned(),
        plan: remote.plan.clone(),
    };
    let settle = || {
        if let Ok(_lock) = RecordLock::take(&file)
            && std::fs::read_to_string(&file)
                .ok()
                .and_then(|text| RequestRecord::parse(&text))
                .is_some_and(|record| record.id == id)
        {
            let _ = remove_record(&file);
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
            request_id(&dir, node, seat, &remote(task), |_| Ok(false))
                .unwrap()
                .0
                .id
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
        let (first, file) =
            request_id(&dir, "n1", Some("s1"), &remote("task"), |_| Ok(false)).unwrap();
        std::fs::write(&file, b"").unwrap();
        let ids: Vec<uuid::Uuid> = std::thread::scope(|threads| {
            let runs: Vec<_> = (0..8)
                .map(|_| {
                    threads.spawn(|| {
                        request_id(&dir, "n1", Some("s1"), &remote("task"), |_| Ok(false))
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
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(
            RequestRecord::parse(&text),
            Some(RequestRecord {
                id: ids[0],
                journaled: Some("local:7".into())
            })
        );
        // A launch whose record a later run replaced neither settles nor
        // overwrites it.
        let stale = Launched {
            id: uuid::Uuid::from_u128(1),
            seat: "s".into(),
            pane: "p".into(),
            file: file.clone(),
        };
        assert!(stale.record_journaled("local:8").is_err());
        stale.settle();
        assert!(file.exists());
        launched.settle();
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Kills: a rerun of a command whose follow-on handoff journaled handing the
    // task off a second time (it must resume through `retry`), and a record
    // whose handoff completed blocking the next run of the command.
    #[test]
    fn a_journaled_handoff_resumes_through_retry_until_it_completes() {
        let dir = crate::test_support::scratch::temp_path(format!(
            "ht-remote-launch-journaled-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let (record, file) =
            request_id(&dir, "n1", Some("s1"), &remote("task"), |_| Ok(false)).unwrap();
        Launched {
            seat: "s".into(),
            pane: "p".into(),
            id: record.id,
            file: file.clone(),
        }
        .record_journaled("local:9")
        .unwrap();
        let Err(RunError::Api(error)) = request_id(&dir, "n1", Some("s1"), &remote("task"), |r| {
            Ok(r == "local:9")
        }) else {
            panic!("a pending journaled handoff refuses")
        };
        assert_eq!(error.code, ErrorCode::Conflict);
        assert!(
            error.detail.contains("herdr-threads retry local:9"),
            "{error:?}"
        );
        let (fresh, _) =
            request_id(&dir, "n1", Some("s1"), &remote("task"), |_| Ok(false)).unwrap();
        assert_ne!(
            fresh.id, record.id,
            "a completed handoff makes this a new command"
        );
        assert_eq!(fresh.journaled, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Kills: a journaled-then-failed handoff settled (a rerun would hand the
    // task off again) or a transient pre-journal failure settled (a rerun
    // would launch a second agent).
    #[test]
    fn the_handoff_outcome_decides_the_launch_request() {
        let api = |code| Err(RunError::Api(ApiError::new(code, "x")));
        assert_eq!(after_handoff(&Ok(()), None), AfterHandoff::Settle);
        assert_eq!(
            after_handoff(&Ok(()), Some("local:3")),
            AfterHandoff::Settle
        );
        assert_eq!(
            after_handoff(&api(ErrorCode::HostUnavailable), Some("local:3")),
            AfterHandoff::KeepJournaled("local:3".into())
        );
        assert_eq!(
            after_handoff(&api(ErrorCode::InvalidRequest), Some("local:3")),
            AfterHandoff::KeepJournaled("local:3".into())
        );
        assert_eq!(
            after_handoff(&api(ErrorCode::InvalidRequest), None),
            AfterHandoff::SettleRefused
        );
        assert_eq!(
            after_handoff(&api(ErrorCode::HostUnavailable), None),
            AfterHandoff::Keep
        );
    }

    /// Resolves the machine to `resolved` and answers every launch poll as
    /// `answering`, recording the machine each poll names.
    struct PollClient {
        resolved: String,
        answering: String,
        polled: std::sync::Mutex<Vec<String>>,
    }

    impl crate::ports::LocalClient for PollClient {
        fn call(
            &self,
            command: crate::protocol::commands::Command,
            _: &CallBudget,
        ) -> Result<crate::protocol::results::CommandResult, ApiError> {
            use crate::protocol::sync_admin::{
                LaunchReply, MachineRow, RemoteLaunchAnswer, SyncCommand, SyncReply,
            };
            let row = |uuid: &str| MachineRow {
                uuid: uuid.into(),
                tag: "Q0V".into(),
                host: None,
            };
            let reply = match command {
                crate::protocol::commands::Command::Sync(SyncCommand::ResolveMachine {
                    ..
                }) => LaunchReply::Remote {
                    node: row(&self.resolved),
                    answer: RemoteLaunchAnswer::Unknown,
                },
                crate::protocol::commands::Command::Sync(SyncCommand::RemoteLaunch {
                    machine,
                    ..
                }) => {
                    self.polled.lock().unwrap().push(machine);
                    LaunchReply::Remote {
                        node: row(&self.answering),
                        answer: RemoteLaunchAnswer::Answered {
                            state: crate::sync::frame::LaunchState::Launched {
                                seat: "sQ0Vseat0001".into(),
                                pane: "w1:p2".into(),
                            },
                        },
                    }
                }
                other => panic!("unexpected {other:?}"),
            };
            Ok(crate::protocol::results::CommandResult::Sync(
                SyncReply::Launch(reply),
            ))
        }

        fn call_with_output(
            &self,
            command: crate::protocol::commands::Command,
            _: &crate::protocol::output::OutputSpec,
            budget: &CallBudget,
        ) -> Result<crate::protocol::results::CommandResult, ApiError> {
            self.call(command, budget)
        }
    }

    // Kills: a launch polled by the typed machine name (a node-table change
    // mid-poll retargets it), and an answer from another node settling the
    // request as if it were this launch's.
    #[test]
    fn a_launch_is_polled_by_its_resolved_node_and_only_that_node_settles_it() {
        let dir = crate::test_support::scratch::temp_path(format!(
            "ht-remote-launch-poll-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let resolved = "ab".repeat(16);
        let other = "cd".repeat(16);
        let client = PollClient {
            resolved: resolved.clone(),
            answering: other,
            polled: std::sync::Mutex::new(Vec::new()),
        };
        let Err(RunError::Api(error)) = run_remote(
            &remote("task"),
            Some("s1"),
            &dir,
            &client,
            &crate::app::SystemClock::new(),
            &mut Vec::new(),
        ) else {
            panic!("another node's answer refuses")
        };
        assert_eq!(error.code, ErrorCode::HostUnavailable);
        assert!(error.detail.contains("no longer the node"), "{error:?}");
        assert_eq!(*client.polled.lock().unwrap(), std::slice::from_ref(&resolved));
        let (record, _) =
            request_id(&dir, &resolved, Some("s1"), &remote("task"), |_| Ok(false)).unwrap();
        let kept = std::fs::read_dir(dir.join("remote-launch"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".id"))
            .count();
        assert_eq!(kept, 1, "the request was not settled");
        let client = PollClient {
            resolved: resolved.clone(),
            answering: resolved.clone(),
            polled: std::sync::Mutex::new(Vec::new()),
        };
        let launched = run_remote(
            &remote("task"),
            Some("s1"),
            &dir,
            &client,
            &crate::app::SystemClock::new(),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(launched.id, record.id, "the rerun resumed the same request");
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
