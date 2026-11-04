//! `internal peer-target-create` (alias `peer-tab-create`): the tab or terminal a
//! peer-requested launch opens (remote launch, TRUST-POLICY N15).
//!
//! Run only by this node's daemon, as a child with a scrubbed environment, for a
//! `launch_request` it recorded in `peer_requests`; on a Herdr server the launch itself
//! is then an ordinary `launch --pane` child, on a herdr-core the terminal starts the
//! gate argv. It creates one target through the `--host-endpoint` host's
//! `CreateTargetPort` (witnessed by that host, never focused) and prints
//! `{"pane":..,"tab":..}` for a Herdr tab or `{"terminal":..,"created":<CreatedTarget>}` for a
//! herdr-core terminal. It reads and
//! writes no seat, binding or journal and claims no caller.

use std::{io::Write, path::Path, sync::Arc, time::Duration};

use super::RunError;
use crate::store::{handoff_obligations::OBLIGATION_TTL_MS, peer_requests::MAX_PLAN_BYTES};
use crate::{
    ports::{
        CreateTargetOutcome, CreateTargetPort, CreateTargetRequest, HostCallContext, HostWitness,
        Placement,
    },
    protocol::{
        handoff::LaunchDirectoryContext,
        ids::{HostCallId, HostTargetId},
        results::{ApiError, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
};

/// Budget of the witness read and the creation together (ms).
const BUDGET_MS: u64 = 30_000;

/// Where `internal peer-target-create` puts its target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetSpec {
    /// A new tab in this workspace (a Herdr workspace or an hc app space).
    Tab(String),
    /// A new terminal with no parent.
    Terminal,
}

/// Creates the target on the host at `endpoint` and prints it.
// Allowed: the entry point carries the dispatcher's pieces, as the other launch runners do.
#[allow(clippy::too_many_arguments)]
pub fn create_target<W: Write>(
    endpoint: &Path,
    spec: &TargetSpec,
    label: &str,
    cwd: &Path,
    gate_argv: Vec<String>,
    directory_context: Option<&LaunchDirectoryContext>,
    clock: Arc<dyn Clock>,
    out: &mut W,
) -> Result<(), RunError> {
    let budget = CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0.saturating_add(BUDGET_MS)),
        cancellation: Cancellation::default(),
    };
    let creator =
        crate::host::target_creator::connect(endpoint, gate_argv, Arc::clone(&clock), &budget)?;
    create_with(
        creator.port.as_ref(),
        creator.witness,
        spec,
        label,
        cwd,
        directory_context,
        budget,
        out,
    )
}

/// [`create_target`] against a given port: generic over the host.
#[allow(clippy::too_many_arguments)]
pub fn create_with<P: CreateTargetPort + ?Sized, W: Write>(
    port: &P,
    witness: HostWitness,
    spec: &TargetSpec,
    label: &str,
    cwd: &Path,
    directory_context: Option<&LaunchDirectoryContext>,
    budget: CallBudget,
    out: &mut W,
) -> Result<(), RunError> {
    let invalid = |detail: &'static str| ApiError::new(ErrorCode::InvalidRequest, detail);
    if directory_context.is_some() && matches!(spec, TargetSpec::Terminal) {
        return Err(invalid("a directory context needs a tab").into());
    }
    let placement = match spec {
        TargetSpec::Tab(workspace) => Placement::NewTab {
            space: HostTargetId::parse(workspace.clone()).map_err(invalid)?,
        },
        TargetSpec::Terminal => Placement::NewTerminal {
            parent: HostTargetId::parse(crate::host::hc::NO_PARENT).map_err(invalid)?,
        },
    };
    let request = CreateTargetRequest {
        correlation: HostCallId::new(uuid::Uuid::new_v4().to_string()),
        placement,
        cwd: cwd.to_path_buf(),
        label: label.to_owned(),
        focus: false,
        env: directory_context.map(|c| c.env.clone()).unwrap_or_default(),
        directory_context: directory_context.cloned(),
        expected_witness: witness,
        gate_argv: Vec::new(),
    };
    let context = HostCallContext {
        budget,
        expected_boot: None,
        expected_epoch: None,
    };
    match port.create_target(&request, &context) {
        CreateTargetOutcome::Created(created) => {
            let line = match &created.root_pane {
                Some(pane) => serde_json::json!({
                    "pane": pane.as_str(),
                    "tab": created.target.as_str(),
                }),
                // The terminal's full creation record: the daemon's worker records it for
                // the launch gate (N15).
                None => serde_json::json!({
                    "terminal": created.target.as_str(),
                    "created": &created,
                }),
            };
            writeln!(out, "{line}")?;
            Ok(())
        }
        CreateTargetOutcome::NotSubmitted(error) | CreateTargetOutcome::OutcomeUnknown(error) => {
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

impl RemoteLaunch {
    /// Whether this launch carries a signed-draft handoff message (an atomic remote handoff).
    pub fn is_handoff(&self) -> bool {
        self.plan.handoff().is_some()
    }
}

/// Refuses a handoff `plan` whose final serialized form would exceed [`MAX_PLAN_BYTES`]: the
/// target stores and bounds the plan after the sender's daemon fills the block, so the CLI
/// measures it with every filled field at its final size.
pub fn plan_size_precheck(plan: &crate::sync::frame::LaunchPlan) -> Result<(), ApiError> {
    if final_plan_len(plan) > MAX_PLAN_BYTES {
        return Err(ApiError::new(
            ErrorCode::InvalidRequest,
            "launch plan too large",
        ));
    }
    Ok(())
}

/// The serialized length `plan` has once the sender's daemon filled and signed its block: each
/// filled field at its longest.
fn final_plan_len(plan: &crate::sync::frame::LaunchPlan) -> usize {
    let mut sized = plan.clone();
    if let Some(block) = sized.message.as_mut() {
        block.v = 1;
        block.request = uuid::Uuid::nil();
        block.sender_node = Some(crate::node::NodeUuid::from_bytes([0; 16]));
        block.author = Some(crate::protocol::ids::SeatId::new(
            "s".repeat(PLACEHOLDER_ID_LEN),
        ));
        block.author_role = Some(crate::protocol::summary::AuthorRole::Service);
        block.message = Some(crate::protocol::ids::MessageId::new(
            "m".repeat(PLACEHOLDER_ID_LEN),
        ));
        block.requested_at = i64::MAX;
        block.sig = "0".repeat(128);
    }
    serde_json::to_vec(&sized).map_or(usize::MAX, |bytes| bytes.len())
}

/// The length of a public id (`<prefix><tag><8 base62 chars>`): a seat or a message.
const PLACEHOLDER_ID_LEN: usize = 12;

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
/// task, another seat) never does. A handoff plan hashes under the `v4` tag; a plain launch
/// keeps `v3`, so a pending 2.x request file is still found after an upgrade.
fn request_key(node_uuid: &str, requester_seat: Option<&str>, remote: &RemoteLaunch) -> String {
    use sha2::Digest;
    let mut hash = sha2::Sha256::new();
    hash.update(if remote.is_handoff() {
        b"herdr-threads remote launch v4\0"
    } else {
        b"herdr-threads remote launch v3\0"
    });
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
    /// Empty when the sender's ledger answered a handoff (it does not hold the pane).
    pub pane: String,
    /// A handoff's message id and 1:1 thread; `None` for a bare launch.
    pub handoff: Option<HandoffIds>,
    id: uuid::Uuid,
    file: std::path::PathBuf,
}

/// The message a handoff signed and the pair thread it lives in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffIds {
    pub thread: String,
    pub message: String,
}

impl Launched {
    /// The result of a started handoff on `machine`: text
    /// `handoff seat=<s> pane=<p>@<machine> thread=<t> message=<m>` (`pane=` omitted when the
    /// ledger answered, `message=` when the answer names no message), or the JSON envelope.
    pub fn handoff_result(&self, machine: &str, json: bool) -> Option<String> {
        let ids = self.handoff.as_ref()?;
        let pane = (!self.pane.is_empty()).then_some(&self.pane);
        let message = (!ids.message.is_empty()).then_some(ids.message.as_str());
        Some(if json {
            format!(
                "{}\n",
                serde_json::json!({ "handoff": {
                    "outcome": "started",
                    "seat": self.seat,
                    "pane": pane,
                    "machine": machine,
                    "thread": ids.thread,
                    "message": message,
                }})
            )
        } else {
            let pane = pane.map_or_else(String::new, |pane| format!(" pane={pane}@{machine}"));
            let message = message.map_or_else(String::new, |message| format!(" message={message}"));
            format!(
                "handoff seat={}{pane} thread={}{message}\n",
                self.seat, ids.thread
            )
        })
    }
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

/// What a CLI says to a daemon that lacks [`HANDOFF_ATOMIC_REMOTE_V1`] (43 bytes).
const NO_ATOMIC_REMOTE: &str = "daemon predates remote handoffs; restart it";

/// Removes request `file` when it still names `id` (a later run of the same command may have
/// replaced it).
fn settle_request(file: &Path, id: uuid::Uuid) {
    if let Ok(_lock) = RecordLock::take(file)
        && std::fs::read_to_string(file)
            .ok()
            .and_then(|text| RequestRecord::parse(&text))
            .is_some_and(|record| record.id == id)
    {
        let _ = remove_record(file);
    }
}

/// Whether request `file` was minted more than [`OBLIGATION_TTL_MS`] ago (its mtime: the file is
/// written once, when its id is minted), so the sender's ledger holds no live row for it.
fn request_file_stale(file: &Path) -> bool {
    std::fs::metadata(file)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age.as_millis() > OBLIGATION_TTL_MS as u128)
}

/// Runs a remote launch to its outcome: the launched seat (now held here as
/// that machine's) and its pane, or a refusal. A handoff plan also needs the
/// caller's `claim`, and ends in the started handoff (seat, pane when known,
/// thread, message) or the sender ledger's answer.
pub fn run_remote<C: crate::ports::LocalClient + ?Sized, W: Write>(
    remote: &RemoteLaunch,
    requester_seat: Option<&str>,
    claim: Option<&crate::protocol::authority::CallerClaim>,
    node_dir: &Path,
    client: &C,
    clock: &dyn Clock,
    stderr: &mut W,
) -> Result<Launched, RunError> {
    run_remote_timed(
        remote,
        requester_seat,
        claim,
        node_dir,
        client,
        clock,
        stderr,
        (launch_timeout(), POLL),
    )
}

/// [`run_remote`] with its `(timeout, poll)` pacing given (tests shorten both).
#[allow(clippy::too_many_arguments)]
fn run_remote_timed<C: crate::ports::LocalClient + ?Sized, W: Write>(
    remote: &RemoteLaunch,
    requester_seat: Option<&str>,
    claim: Option<&crate::protocol::authority::CallerClaim>,
    node_dir: &Path,
    client: &C,
    clock: &dyn Clock,
    stderr: &mut W,
    (timeout, poll): (Duration, Duration),
) -> Result<Launched, RunError> {
    use crate::protocol::ids::{SeatId, direct_thread_id};
    use crate::protocol::sync_admin::{
        HandoffLedgerOutcome, LaunchReply, RemoteLaunchAnswer, SyncCommand, SyncReply,
    };
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
    let handoff = remote.is_handoff();
    if handoff
        && !client.supports_capability(
            crate::protocol::capabilities::HANDOFF_ATOMIC_REMOTE_V1,
            &budget(),
        )
    {
        return Err(ApiError::new(ErrorCode::Unsupported, NO_ATOMIC_REMOTE).into());
    }
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
    let (mut record, mut file) =
        request_id(node_dir, &node.uuid, requester_seat, remote, |reference| {
            intent_pending(node_dir, reference)
        })?;
    let mut id = record.id;
    // A request file older than the ledger's lifetime is only asked about, never signed again.
    let mut resume_only = handoff && request_file_stale(&file);
    let mut plan = remote.plan.clone();
    if let (Some(block), Some(seat)) = (plan.message.as_mut(), requester_seat) {
        block.author = Some(SeatId::new(seat));
    }
    let request_of = |id| LaunchRequest {
        id,
        requester_seat: requester_seat.unwrap_or_default().to_owned(),
        plan: plan.clone(),
    };
    let mut request = request_of(id);
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
                claim: if handoff { claim.cloned() } else { None },
                resume_only,
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
        let (node, answer, handoff_message) = match reply {
            LaunchReply::Remote {
                node,
                answer,
                handoff_message,
            } if &node.uuid == resolved => (node, answer, handoff_message),
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
                    let ids = handoff.then(|| HandoffIds {
                        thread: direct_thread_id(
                            &SeatId::new(requester_seat.unwrap_or_default()),
                            &SeatId::new(&seat),
                        )
                        .map(|thread| thread.as_str().to_owned())
                        .unwrap_or_default(),
                        message: handoff_message.unwrap_or_default(),
                    });
                    if !handoff {
                        writeln!(
                            stderr,
                            "launched: {} on {}@{} (request {})",
                            one_line(&seat),
                            one_line(&pane),
                            node.tag,
                            &id.to_string()[..8]
                        )?;
                    }
                    return Ok(Launched {
                        seat,
                        pane,
                        handoff: ids,
                        id,
                        file: file.clone(),
                    });
                }
                LaunchState::Refused { detail } => {
                    settle_request(&file, id);
                    return Err(refuse(
                        ErrorCode::InvalidRequest,
                        format!("{label} refused the launch: {}", one_line(&detail)),
                    ));
                }
                LaunchState::Failed {
                    detail,
                    pane,
                    unconfirmed,
                    staged,
                } => {
                    // An unconfirmed start keeps the id: a rerun reports the same, never a
                    // second agent in a pane that may already run one.
                    if !unconfirmed {
                        settle_request(&file, id);
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
                    // The target staged the message before it failed: it stands there.
                    let staged = staged.map_or_else(String::new, |staged| {
                        format!(
                            "; message {} stands in thread {}",
                            one_line(&staged.message),
                            one_line(&staged.thread)
                        )
                    });
                    return Err(refuse(
                        ErrorCode::HostUnavailable,
                        format!(
                            "launch on {label} failed: {}{left}{escape}{staged}",
                            one_line(&detail)
                        ),
                    ));
                }
                LaunchState::Running { .. } => "running".to_owned(),
                LaunchState::Unavailable { reason } => one_line(&reason),
                LaunchState::Unknown => "unknown answer".to_owned(),
            },
            RemoteLaunchAnswer::NoCap { version, cap } => {
                settle_request(&file, id);
                let cap = if cap.is_empty() {
                    crate::sync::frame::PEER_LAUNCH_CAP
                } else {
                    cap.as_str()
                };
                let version = version.as_deref().unwrap_or("unknown");
                return Err(refuse(
                    ErrorCode::Unsupported,
                    if cap == crate::sync::frame::PEER_HANDOFF_CAP {
                        format!(
                            "machine {label} cannot take a handoff (needs {cap}; version \
                             {version}); upgrade it to 3.0.0 or later"
                        )
                    } else {
                        format!(
                            "machine {label} cannot launch for other machines (needs {cap}; \
                             version {version})"
                        )
                    },
                ));
            }
            RemoteLaunchAnswer::Unreachable { .. } => "unreachable".to_owned(),
            RemoteLaunchAnswer::Timeout { secs } => format!("no answer in {secs}s"),
            RemoteLaunchAnswer::NotReplicated { .. } => "launched; waiting for its seat".to_owned(),
            // The sender's ledger answered; each terminal answer settles the request file.
            RemoteLaunchAnswer::HandoffLedger { outcome } => match outcome {
                HandoffLedgerOutcome::Linked {
                    seat,
                    thread,
                    message,
                } => {
                    return Ok(Launched {
                        seat,
                        pane: String::new(),
                        handoff: Some(HandoffIds { thread, message }),
                        id,
                        file: file.clone(),
                    });
                }
                HandoffLedgerOutcome::Expired { seat } => {
                    settle_request(&file, id);
                    let req = &id.to_string()[..8];
                    return Err(refuse(
                        ErrorCode::HostUnavailable,
                        match seat {
                            None => format!("handoff {req} to {label} expired unconfirmed"),
                            Some(seat) => format!(
                                "handoff {req} to {label} expired unlinked: seat {}",
                                one_line(&seat)
                            ),
                        },
                    ));
                }
                // No row for a stale request file: it was pruned. That command is over; this
                // run is a new one under a new id, never the old id signed again.
                HandoffLedgerOutcome::Unknown if resume_only => {
                    settle_request(&file, id);
                    (record, file) =
                        request_id(node_dir, &node.uuid, requester_seat, remote, |reference| {
                            intent_pending(node_dir, reference)
                        })?;
                    id = record.id;
                    request = request_of(id);
                    resume_only = false;
                    continue;
                }
                HandoffLedgerOutcome::Unknown => "unknown answer".to_owned(),
            },
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
        if started.elapsed() >= timeout {
            let req = &id.to_string()[..8];
            return Err(refuse(
                ErrorCode::HostUnavailable,
                if handoff {
                    format!(
                        "handoff to {label} still pending: request {req} stays queued until it \
                         starts or expires (6 days)"
                    )
                } else {
                    format!(
                        "launch on {label} still pending ({pending}): request {req}; rerun to resume"
                    )
                },
            ));
        }
        std::thread::sleep(poll);
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
                profile_dir: None,
                handoff_prompt: None,
                message: None,
            },
            then: Some(Box::new(super::super::topology_handoff::Request {
                policy: None,
                target: None,
                automatic: true,
                new_tab: None,
                new_terminal: false,
                existing: true,
                selector: Default::default(),
                seat: None,
                cwd: None,
                profile_dir: None,
                thread: None,
                thread_name: None,
                topic: None,
                goal: None,
                summary: None,
                body: task.into(),
                kind: None,
                binary: None,
                name: None,
                argv: Vec::new(),
            })),
        }
    }

    struct Recording(std::sync::Mutex<Vec<CreateTargetRequest>>);

    impl CreateTargetPort for Recording {
        fn create_target(
            &self,
            request: &CreateTargetRequest,
            _: &HostCallContext,
        ) -> CreateTargetOutcome {
            self.0.lock().unwrap().push(request.clone());
            CreateTargetOutcome::NotSubmitted(ApiError::new(ErrorCode::HostUnavailable, "recorded"))
        }
    }

    // Kills: a handoff tab created with an empty env (the agent then runs on the shell's default
    // root while the CLI reports started), a context sent without its env or env without its
    // context, env leaking into a bare tab, and a context accepted for a terminal.
    #[test]
    fn create_with_hands_the_frozen_context_to_the_request() {
        let witness = HostWitness::HcCore {
            core_id: "c".into(),
            peer: crate::ports::HcPeer {
                pid: 1,
                start_seconds: 1,
                uid: 1,
            },
        };
        let budget = || CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Cancellation::default(),
        };
        let context = LaunchDirectoryContext {
            profile_dir: "/p".into(),
            cwd: "/w".into(),
            env: [("PROFILE_ROOT".to_owned(), "/p".to_owned())].into(),
        };
        let port = Recording(Default::default());
        let tab = TargetSpec::Tab("w1".into());
        let run = |spec: &TargetSpec, ctx: Option<&LaunchDirectoryContext>| {
            create_with(
                &port,
                witness.clone(),
                spec,
                "x",
                Path::new("/w"),
                ctx,
                budget(),
                &mut Vec::new(),
            )
        };
        assert!(run(&tab, Some(&context)).is_err());
        assert!(run(&tab, None).is_err());
        assert!(run(&TargetSpec::Terminal, Some(&context)).is_err());
        let seen = port.0.lock().unwrap();
        assert_eq!(seen.len(), 2, "the terminal with a context creates nothing");
        assert_eq!(seen[0].env, context.env);
        assert_eq!(seen[0].directory_context.as_ref(), Some(&context));
        assert!(seen[1].env.is_empty());
        assert_eq!(seen[1].directory_context, None);
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
            handoff: None,
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
            handoff: None,
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
            handoff: None,
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
                    handoff_message: None,
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
                        handoff_message: None,
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
            None,
            &dir,
            &client,
            &crate::app::SystemClock::new(),
            &mut Vec::new(),
        ) else {
            panic!("another node's answer refuses")
        };
        assert_eq!(error.code, ErrorCode::HostUnavailable);
        assert!(error.detail.contains("no longer the node"), "{error:?}");
        assert_eq!(
            *client.polled.lock().unwrap(),
            std::slice::from_ref(&resolved)
        );
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
            None,
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

    // ---- atomic remote handoff (ht-8lem.10) ----

    fn handoff_remote(body: &str, summary: &str, profile: Option<&str>) -> RemoteLaunch {
        let mut remote = remote("unused");
        remote.then = None;
        remote.plan.handoff_prompt = Some(summary.into());
        remote.plan.profile_dir = profile.map(str::to_owned);
        remote.plan.message = Some(Box::new(crate::sync::handoff_block::SenderBlock {
            body: body.into(),
            priority: 0,
            require_ack: true,
            ..Default::default()
        }));
        remote
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            crate::test_support::scratch::temp_path(format!("ht-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A daemon double: answers `ResolveMachine`, then each `RemoteLaunch` poll with the next
    /// scripted `(answer, handoff_message)` (the last repeats), recording every command.
    struct Script {
        caps: bool,
        node: String,
        answers: std::sync::Mutex<
            std::collections::VecDeque<(
                crate::protocol::sync_admin::RemoteLaunchAnswer,
                Option<String>,
            )>,
        >,
        commands: std::sync::Mutex<Vec<crate::protocol::commands::Command>>,
    }

    impl Script {
        fn new(
            caps: bool,
            answers: Vec<(
                crate::protocol::sync_admin::RemoteLaunchAnswer,
                Option<String>,
            )>,
        ) -> Self {
            Self {
                caps,
                node: "ab".repeat(16),
                answers: std::sync::Mutex::new(answers.into()),
                commands: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn launches(
            &self,
        ) -> Vec<(
            Box<crate::sync::frame::LaunchRequest>,
            Option<crate::protocol::authority::CallerClaim>,
            bool,
        )> {
            self.commands
                .lock()
                .unwrap()
                .iter()
                .filter_map(|command| match command {
                    crate::protocol::commands::Command::Sync(
                        crate::protocol::sync_admin::SyncCommand::RemoteLaunch {
                            request,
                            claim,
                            resume_only,
                            ..
                        },
                    ) => Some((request.clone(), claim.clone(), *resume_only)),
                    _ => None,
                })
                .collect()
        }
    }

    impl crate::ports::LocalClient for Script {
        fn supports_capability(&self, _: &str, _: &CallBudget) -> bool {
            self.caps
        }

        fn call(
            &self,
            command: crate::protocol::commands::Command,
            _: &CallBudget,
        ) -> Result<crate::protocol::results::CommandResult, ApiError> {
            use crate::protocol::sync_admin::{LaunchReply, MachineRow, RemoteLaunchAnswer};
            let node = MachineRow {
                uuid: self.node.clone(),
                tag: "Q0V".into(),
                host: Some("laptop".into()),
            };
            self.commands.lock().unwrap().push(command.clone());
            let (answer, handoff_message) = match &command {
                crate::protocol::commands::Command::Sync(
                    crate::protocol::sync_admin::SyncCommand::RemoteLaunch { .. },
                ) => {
                    let mut answers = self.answers.lock().unwrap();
                    if answers.len() > 1 {
                        answers.pop_front().unwrap()
                    } else {
                        answers.front().cloned().unwrap()
                    }
                }
                _ => (RemoteLaunchAnswer::Unknown, None),
            };
            Ok(crate::protocol::results::CommandResult::Sync(
                crate::protocol::sync_admin::SyncReply::Launch(LaunchReply::Remote {
                    node,
                    answer,
                    handoff_message,
                }),
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

    fn claim_of(seat: &str) -> crate::protocol::authority::CallerClaim {
        use crate::protocol::{
            authority::{CallerClaim, CallerRole, Harness},
            ids::{ExecutionId, HostLocator, HostTargetId, NativeSessionId, SeatId},
        };
        CallerClaim {
            instance: "i".into(),
            host: HostLocator::parse("/claim.sock").unwrap(),
            seat: SeatId::new(seat),
            binding_generation: 1,
            role: CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: NativeSessionId::new("n"),
            execution: ExecutionId::new("e"),
            target: HostTargetId::new("w1:p1"),
            agent_id: None,
            liveness: None,
        }
    }

    fn answered(
        state: crate::sync::frame::LaunchState,
    ) -> crate::protocol::sync_admin::RemoteLaunchAnswer {
        crate::protocol::sync_admin::RemoteLaunchAnswer::Answered { state }
    }

    /// Runs `remote` against `script` (fast pacing) in a fresh node dir.
    fn drive(
        remote: &RemoteLaunch,
        script: &Script,
        dir: &Path,
        timeout_ms: u64,
    ) -> (Result<Launched, RunError>, String) {
        let mut stderr = Vec::new();
        let claim = claim_of("sQ0Vsender001");
        let result = run_remote_timed(
            remote,
            Some("sQ0Vsender001"),
            Some(&claim),
            dir,
            script,
            &crate::app::SystemClock::new(),
            &mut stderr,
            (Duration::from_millis(timeout_ms), Duration::from_millis(1)),
        );
        (result, String::from_utf8(stderr).unwrap())
    }

    fn detail(result: Result<Launched, RunError>) -> (ErrorCode, String) {
        match result {
            Err(RunError::Api(error)) => (error.code, error.detail),
            other => panic!("expected an api error, got {other:?}"),
        }
    }

    const MSG: &str = "mQ0Vabcd1234";

    // Kills: the v3 tag surviving (an old request file adopted by a handoff plan), and a key
    // that ignores the draft (two different tasks, summaries or profile dirs sharing an id).
    #[test]
    fn request_key_v4() {
        use sha2::Digest;
        let base = handoff_remote("task", "sum", Some("/p"));
        let key = request_key("n1", Some("s1"), &base);
        let mut hash = sha2::Sha256::new();
        hash.update(b"herdr-threads remote launch v4\0n1\0s1\0");
        hash.update(serde_json::to_vec(&base.plan).unwrap());
        hash.update(b"\0");
        hash.update(serde_json::to_vec(&base.then).unwrap());
        let expected: String = hash
            .finalize()
            .iter()
            .take(16)
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(key, expected);
        for other in [
            handoff_remote("task 2", "sum", Some("/p")),
            handoff_remote("task", "sum 2", Some("/p")),
            handoff_remote("task", "sum", Some("/q")),
            handoff_remote("task", "sum", None),
        ] {
            assert_ne!(key, request_key("n1", Some("s1"), &other));
        }
    }

    // Kills: the v4 tag on a plain launch, which would rename the key of a pending 2.x request
    // file after an upgrade and launch twice.
    #[test]
    fn request_key_v3_for_a_plain_launch() {
        use sha2::Digest;
        let mut bare = remote("ignored");
        bare.then = None;
        assert!(!bare.is_handoff());
        let key = request_key("n1", Some("s1"), &bare);
        let mut hash = sha2::Sha256::new();
        hash.update(b"herdr-threads remote launch v3\0n1\0s1\0");
        hash.update(serde_json::to_vec(&bare.plan).unwrap());
        hash.update(b"\0");
        hash.update(serde_json::to_vec(&bare.then).unwrap());
        let expected: String = hash
            .finalize()
            .iter()
            .take(16)
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(key, expected);
    }

    // Kills: a handoff sent to a daemon that cannot take a claim (it would drop the claim and
    // sign nothing), a different refusal text, and anything sent before the refusal.
    #[test]
    fn daemon_without_atomic_remote_cap() {
        let dir = scratch("handoff-nocap");
        let script = Script::new(false, vec![]);
        let (result, _) = drive(&handoff_remote("t", "s", None), &script, &dir, 1000);
        let (code, text) = detail(result);
        assert_eq!(code, ErrorCode::Unsupported);
        assert_eq!(text, "daemon predates remote handoffs; restart it");
        assert_eq!(text.len(), 43);
        assert!(script.commands.lock().unwrap().is_empty(), "nothing sent");
        // A bare launch needs no such capability.
        let bare = Script::new(
            false,
            vec![(
                answered(crate::sync::frame::LaunchState::Launched {
                    seat: "sQ0Vseat0001".into(),
                    pane: "w1:p2".into(),
                }),
                None,
            )],
        );
        assert!(drive(&remote("t"), &bare, &dir, 1000).0.is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Kills: a claim not sent (the sender's daemon refuses it), sent on a bare launch, the
    // draft's author left unset or set to another seat, and a polled id that changes.
    #[test]
    fn sends_claim_and_sets_the_author() {
        let dir = scratch("handoff-claim");
        let launched = answered(crate::sync::frame::LaunchState::Launched {
            seat: "sQ0Vseat0001".into(),
            pane: "w1:p2".into(),
        });
        let script = Script::new(true, vec![(launched, Some(MSG.into()))]);
        let hand = handoff_remote("t", "s", None);
        assert_eq!(hand.plan.message.as_ref().unwrap().author, None);
        let (result, _) = drive(&hand, &script, &dir, 1000);
        assert!(result.is_ok());
        let sent = script.launches();
        let [(request, claim, resume_only)] = sent.as_slice() else {
            panic!("one poll: {sent:?}")
        };
        assert_eq!(claim, &Some(claim_of("sQ0Vsender001")));
        assert!(!resume_only);
        assert_eq!(request.requester_seat, "sQ0Vsender001");
        let block = request.plan.message.as_ref().unwrap();
        assert_eq!(
            block.author.as_ref().map(|a| a.as_str()),
            Some("sQ0Vsender001")
        );
        assert_eq!(block.sig, "");
        // A bare launch carries no claim.
        let bare = Script::new(
            true,
            vec![(
                answered(crate::sync::frame::LaunchState::Launched {
                    seat: "sQ0Vseat0001".into(),
                    pane: "w1:p2".into(),
                }),
                None,
            )],
        );
        assert!(drive(&remote("t2"), &bare, &dir, 1000).0.is_ok());
        assert_eq!(bare.launches()[0].1, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Kills: a result line that drops the thread or message, a thread other than the sender's
    // pair with the launched seat, a JSON envelope other than {"handoff":{..}}, and a ledger
    // `linked` answer that prints a pane it does not hold.
    #[test]
    fn started_handoff_results() {
        let dir = scratch("handoff-result");
        let script = Script::new(
            true,
            vec![(
                answered(crate::sync::frame::LaunchState::Launched {
                    seat: "sQ0Vseat0001".into(),
                    pane: "w1:p2".into(),
                }),
                Some(MSG.into()),
            )],
        );
        let remote = handoff_remote("t", "s", None);
        let (result, _) = drive(&remote, &script, &dir, 1000);
        let launched = result.unwrap();
        let thread = crate::protocol::ids::direct_thread_id(
            &crate::protocol::ids::SeatId::new("sQ0Vsender001"),
            &crate::protocol::ids::SeatId::new("sQ0Vseat0001"),
        )
        .unwrap();
        let text = launched.handoff_result("Q0V", false).unwrap();
        assert_eq!(
            text,
            format!(
                "handoff seat=sQ0Vseat0001 pane=w1:p2@Q0V thread={} message={MSG}\n",
                thread.as_str()
            )
        );
        let json: serde_json::Value =
            serde_json::from_str(&launched.handoff_result("Q0V", true).unwrap()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"handoff": {
                "outcome": "started", "seat": "sQ0Vseat0001", "pane": "w1:p2",
                "machine": "Q0V", "thread": thread.as_str(), "message": MSG,
            }})
        );
        launched.settle();
        assert!(
            std::fs::read_dir(dir.join("remote-launch"))
                .unwrap()
                .filter_map(|e| e.ok())
                .all(|e| !e.file_name().to_string_lossy().ends_with(".id")),
            "the started handoff settled its request file"
        );
        // The ledger's `linked` answer: no pane.
        use crate::protocol::sync_admin::{HandoffLedgerOutcome, RemoteLaunchAnswer};
        let linked = Script::new(
            true,
            vec![(
                RemoteLaunchAnswer::HandoffLedger {
                    outcome: HandoffLedgerOutcome::Linked {
                        seat: "sQ0Vseat0001".into(),
                        thread: "tQ0Vthread001".into(),
                        message: MSG.into(),
                    },
                },
                Some(MSG.into()),
            )],
        );
        let launched = drive(&remote, &linked, &dir, 1000).0.unwrap();
        assert_eq!(
            launched.handoff_result("Q0V", false).unwrap(),
            format!("handoff seat=sQ0Vseat0001 thread=tQ0Vthread001 message={MSG}\n")
        );
        let json: serde_json::Value =
            serde_json::from_str(&launched.handoff_result("Q0V", true).unwrap()).unwrap();
        assert_eq!(json["handoff"]["pane"], serde_json::Value::Null);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Kills: an empty ` message=` (and `"message": ""`) when the target's answer names no message.
    #[test]
    fn handoff_result_omits_an_empty_message() {
        let dir = scratch("handoff-nomsg");
        let script = Script::new(
            true,
            vec![(
                answered(crate::sync::frame::LaunchState::Launched {
                    seat: "sQ0Vseat0001".into(),
                    pane: "w1:p2".into(),
                }),
                Some(MSG.into()),
            )],
        );
        let mut launched = drive(&handoff_remote("t", "s", None), &script, &dir, 1000)
            .0
            .unwrap();
        launched.handoff.as_mut().unwrap().message.clear();
        let thread = launched.handoff.as_ref().unwrap().thread.clone();
        assert_eq!(
            launched.handoff_result("Q0V", false).unwrap(),
            format!("handoff seat=sQ0Vseat0001 pane=w1:p2@Q0V thread={thread}\n")
        );
        let json: serde_json::Value =
            serde_json::from_str(&launched.handoff_result("Q0V", true).unwrap()).unwrap();
        assert_eq!(json["handoff"]["message"], serde_json::Value::Null);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Kills: target refusals and failures leaving the exit code or frame, a staged failure that
    // hides the message standing in the pair, an upgrade error with the wrong capability text,
    // other capabilities losing today's text, and a refused/failed request left on disk.
    #[test]
    fn refusals_failures_and_upgrade_error() {
        use crate::protocol::sync_admin::RemoteLaunchAnswer;
        use crate::sync::frame::{LaunchStaged, LaunchState};
        let dir = scratch("handoff-refusals");
        let remote = handoff_remote("t", "s", None);
        let run = |answer: RemoteLaunchAnswer| {
            let script = Script::new(true, vec![(answer, None)]);
            detail(drive(&remote, &script, &dir, 1000).0)
        };
        let kept = || {
            std::fs::read_dir(dir.join("remote-launch"))
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().ends_with(".id"))
                .count()
        };
        let (code, text) = run(answered(LaunchState::Refused {
            detail: "profile_dir is not a directory".into(),
        }));
        assert_eq!(code, ErrorCode::InvalidRequest);
        assert_eq!(
            text,
            "Q0V laptop refused the launch: profile_dir is not a directory"
        );
        assert_eq!(kept(), 0);
        let (code, text) = run(answered(LaunchState::Failed {
            detail: "interrupted".into(),
            pane: None,
            unconfirmed: false,
            staged: Some(LaunchStaged {
                thread: "tQ0Vthread001".into(),
                message: MSG.into(),
            }),
        }));
        assert_eq!(code, ErrorCode::HostUnavailable);
        assert_eq!(
            text,
            format!(
                "launch on Q0V laptop failed: interrupted; message {MSG} stands in thread \
                 tQ0Vthread001"
            )
        );
        assert_eq!(kept(), 0);
        let (_, text) = run(answered(LaunchState::Failed {
            detail: "interrupted".into(),
            pane: None,
            unconfirmed: false,
            staged: None,
        }));
        assert_eq!(text, "launch on Q0V laptop failed: interrupted");
        let (code, text) = run(RemoteLaunchAnswer::NoCap {
            version: Some("2.9.0".into()),
            cap: "peer.handoff.v1".into(),
        });
        assert_eq!(code, ErrorCode::Unsupported);
        assert_eq!(
            text,
            "machine Q0V laptop cannot take a handoff (needs peer.handoff.v1; version 2.9.0); \
             upgrade it to 3.0.0 or later"
        );
        let (_, text) = run(RemoteLaunchAnswer::NoCap {
            version: None,
            cap: "peer.launch.v1".into(),
        });
        assert_eq!(
            text,
            "machine Q0V laptop cannot launch for other machines (needs peer.launch.v1; \
             version unknown)"
        );
        assert_eq!(kept(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Kills: Unavailable ending the wait early, a poll end that keeps today's "rerun to resume"
    // line for a handoff, and a still-pending request settled (a rerun would mint a new id).
    #[test]
    fn unavailable_keeps_polling_and_the_pending_line_ends_it() {
        use crate::sync::frame::LaunchState;
        let dir = scratch("handoff-pending");
        let remote = handoff_remote("t", "s", None);
        let unavailable = answered(LaunchState::Unavailable {
            reason: "sender node key not replicated here yet".into(),
        });
        let launched = answered(LaunchState::Launched {
            seat: "sQ0Vseat0001".into(),
            pane: "w1:p2".into(),
        });
        let script = Script::new(
            true,
            vec![
                (unavailable.clone(), None),
                (unavailable.clone(), None),
                (launched, Some(MSG.into())),
            ],
        );
        let (result, stderr) = drive(&remote, &script, &dir, 60_000);
        assert!(result.is_ok());
        assert_eq!(script.launches().len(), 3, "polled until it started");
        assert!(stderr.starts_with("launching on Q0V laptop (request "));
        let _ = std::fs::remove_dir_all(&dir);
        let dir = scratch("handoff-pending-end");
        let script = Script::new(true, vec![(unavailable, None)]);
        let (result, _) = drive(&remote, &script, &dir, 0);
        let (code, text) = detail(result);
        assert_eq!(code, ErrorCode::HostUnavailable);
        let (record, _) = request_id(&dir, &script.node, Some("sQ0Vsender001"), &remote, |_| {
            Ok(false)
        })
        .unwrap();
        let req = &record.id.to_string()[..8];
        assert_eq!(
            text,
            format!(
                "handoff to Q0V laptop still pending: request {req} stays queued until it \
                 starts or expires (6 days)"
            )
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Kills: a ledger answer printed as a pending poll (looping to the timeout), an expired row
    // reported as started, and a terminal answer leaving the request file behind.
    #[test]
    fn ledger_answers_print_and_settle() {
        use crate::protocol::sync_admin::{HandoffLedgerOutcome, RemoteLaunchAnswer};
        let dir = scratch("handoff-ledger");
        let remote = handoff_remote("t", "s", None);
        let expired = |seat: Option<&str>| {
            let script = Script::new(
                true,
                vec![(
                    RemoteLaunchAnswer::HandoffLedger {
                        outcome: HandoffLedgerOutcome::Expired {
                            seat: seat.map(str::to_owned),
                        },
                    },
                    None,
                )],
            );
            let (result, _) = drive(&remote, &script, &dir, 1000);
            let (_, record) = (
                (),
                request_id(&dir, &script.node, Some("sQ0Vsender001"), &remote, |_| {
                    Ok(false)
                })
                .unwrap()
                .0,
            );
            (detail(result), record.id.to_string())
        };
        let ((code, text), next) = expired(None);
        assert_eq!(code, ErrorCode::HostUnavailable);
        // The request file was settled, so the id seen now is a fresh one; the text names the
        // settled one.
        let req = text
            .strip_prefix("handoff ")
            .and_then(|t| t.strip_suffix(" to Q0V laptop expired unconfirmed"))
            .unwrap_or_else(|| panic!("{text}"));
        assert_eq!(req.len(), 8);
        assert!(!next.starts_with(req), "a new id after the ledger answer");
        let ((_, text), _) = expired(Some("sQ0Vseat0001"));
        assert!(
            text.starts_with("handoff ")
                && text.ends_with(" to Q0V laptop expired unlinked: seat sQ0Vseat0001"),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Kills: a stale request file signed again (resume_only unset), a fresh file sent as
    // resume-only, and `Unknown` for a stale file not continuing as a new command under a new
    // id (or reusing the old one).
    #[test]
    fn a_stale_request_file_resumes_only_and_unknown_starts_anew() {
        use crate::protocol::sync_admin::{HandoffLedgerOutcome, RemoteLaunchAnswer};
        let dir = scratch("handoff-stale");
        let remote = handoff_remote("t", "s", None);
        let script_for = || {
            Script::new(
                true,
                vec![
                    (
                        RemoteLaunchAnswer::HandoffLedger {
                            outcome: HandoffLedgerOutcome::Unknown,
                        },
                        None,
                    ),
                    (
                        answered(crate::sync::frame::LaunchState::Launched {
                            seat: "sQ0Vseat0001".into(),
                            pane: "w1:p2".into(),
                        }),
                        Some(MSG.into()),
                    ),
                ],
            )
        };
        // A fresh file: not resume-only.
        let fresh = script_for();
        let (result, _) = drive(&remote, &fresh, &dir, 1000);
        let first_id = {
            let sent = fresh.launches();
            assert!(!sent[0].2);
            sent[0].0.id
        };
        drop(result);
        // Age that request file past the ledger's lifetime.
        let (record, file) = request_id(&dir, &fresh.node, Some("sQ0Vsender001"), &remote, |_| {
            Ok(false)
        })
        .unwrap();
        assert_eq!(record.id, first_id);
        let old =
            std::time::SystemTime::now() - Duration::from_millis(OBLIGATION_TTL_MS as u64 + 60_000);
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let stale = script_for();
        let (result, _) = drive(&remote, &stale, &dir, 1000);
        let launched = result.unwrap();
        let sent = stale.launches();
        let ids: Vec<_> = sent.iter().map(|(r, _, resume)| (r.id, *resume)).collect();
        assert_eq!(ids.len(), 2, "{ids:?}");
        assert_eq!(
            ids[0],
            (first_id, true),
            "the stale file was only asked about"
        );
        assert_ne!(ids[1].0, first_id, "Unknown continued under a new id");
        assert!(!ids[1].1, "the new command signs");
        assert!(launched.handoff_result("Q0V", false).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Kills: a placeholder block smaller than the real one (a plan the target then refuses), an
    // off-by-one at the limit, and a placeholder message id not the length of a real one.
    #[test]
    fn plan_size_precheck_measures_the_final_plan() {
        let with_body = |n: usize| handoff_remote(&"a".repeat(n), "s", None).plan;
        let base = final_plan_len(&with_body(0));
        let at_limit = with_body(MAX_PLAN_BYTES - base);
        assert_eq!(final_plan_len(&at_limit), MAX_PLAN_BYTES);
        assert!(plan_size_precheck(&at_limit).is_ok());
        let over = with_body(MAX_PLAN_BYTES - base + 1);
        let error = plan_size_precheck(&over).unwrap_err();
        assert_eq!(error.detail, "launch plan too large");
        // Quotes double when escaped: 65 536 of them already overflow.
        let quotes = handoff_remote(&"\"".repeat(65_536), "s", None).plan;
        assert!(plan_size_precheck(&quotes).is_err());
        // The placeholders are at least as long as a real signed block, and the id lengths are
        // the real ones.
        let tag: crate::node::NodeTag = "Q0V".parse().unwrap();
        let message = crate::protocol::ids::generate_public_id("m", &tag);
        let seat = crate::protocol::ids::generate_public_id("s", &tag);
        assert_eq!(message.len(), PLACEHOLDER_ID_LEN);
        assert_eq!(seat.len(), PLACEHOLDER_ID_LEN);
        let mut real = with_body(100);
        let block = real.message.as_mut().unwrap();
        block.v = 1;
        block.request = uuid::Uuid::new_v4();
        block.sender_node = Some(crate::node::NodeUuid::from_bytes([0xab; 16]));
        block.author = Some(crate::protocol::ids::SeatId::new(seat));
        block.author_role = Some(crate::protocol::summary::AuthorRole::Agent);
        block.message = Some(crate::protocol::ids::MessageId::new(message));
        block.requested_at = 1_788_000_000_000;
        block.sig = "f".repeat(128);
        let real_len = serde_json::to_vec(&real).unwrap().len();
        let sized = final_plan_len(&with_body(100));
        assert!(sized >= real_len, "{sized} < {real_len}");
        // Only the timestamp (19 vs 13 digits) and the role ("service" vs "agent") differ.
        assert_eq!(sized - real_len, (19 - 13) + (7 - 5));
    }
}
