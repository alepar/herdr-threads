//! Launches an agent here for a linked peer node (remote launch, TRUST-POLICY
//! N15, A3 `peer_request:<node_uuid>`).
//!
//! Responsibility (PeerLaunchService)
//! - Owns: the decision to take a peer's `launch_request` (its target exists
//!   on one attached host and is not held, a new tab's workspace and `cwd`
//!   exist here), recording it in `peer_requests` before acting, and running
//!   it as children of this binary with a scrubbed environment: for a new tab
//!   `internal peer-target-create` (label ending `[from <tag>]`), then the
//!   ordinary `launch --pane`, so the launch is exactly a local `launch`.
//!   On a herdr-core the creation child starts the launch-gate shim in the new terminal and
//!   this worker then allocates the seat (an ordinary guarded resolution and a
//!   `managed_launch` binding on the shim's process) and writes the `peer_requests` row's
//!   gate-only record, which releases the shim (N15).
//! - Hides: the child processes and their output.
//! - Must not: act as the requesting seat or as this node's operator, resume
//!   an interrupted request, or launch twice for one id. The child runs with
//!   no caller pane, harness session or Herdr pane environment, so nothing it
//!   writes can be attributed to a stray session; its rows are linked to the
//!   request through the `peer_requests` row (pane and seat).
//! - Role: installed on the sync handle by the daemon; called by the
//!   coordinator on a blocking thread; one worker thread per accepted request.
//!
//! Core and harness-neutral: the harness id is passed through as text.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use crate::node::NodeUuid;
use crate::protocol::address;
use crate::protocol::results::ApiError;
use crate::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant};
use crate::service::fair_writer::FairWriter;
use crate::service::host_sessions::{HostSession, HostSessions};
use crate::store::SqliteStore;
use crate::store::peer_requests::{self, PeerRequest};
use crate::sync::coordinator::remote_launch::PeerLauncher;
use crate::sync::frame::{LaunchRequest, LaunchState, LaunchTarget};

/// Peer-requested launches one requesting node may have running here at once.
pub const MAX_RUNNING_PER_PEER: u64 = 4;
/// Retries of a worker's progress or outcome write after the first fails.
const RECORD_RETRIES: [Duration; 3] = [
    Duration::from_millis(200),
    Duration::from_secs(1),
    Duration::from_secs(5),
];
/// Budget of one store read or write, and of the topology reads (ms).
const CALL_BUDGET_MS: u64 = 3_000;
/// A child still running after this long is killed and the request failed.
const CHILD_TIMEOUT: Duration = Duration::from_secs(180);
/// How often a running child is checked.
const CHILD_POLL: Duration = Duration::from_millis(100);

/// The environment a launch child keeps: what any login shell has, never a
/// caller pane, harness session or Herdr pane variable.
const KEPT_ENV: &[&str] = &[
    "HOME",
    "PATH",
    "USER",
    "LOGNAME",
    "SHELL",
    "LANG",
    "TZ",
    "TMPDIR",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_CACHE_HOME",
    "XDG_RUNTIME_DIR",
];

/// The environment of a launch child, from `vars` (the daemon's own).
pub fn child_env(vars: impl Iterator<Item = (OsString, OsString)>) -> Vec<(OsString, OsString)> {
    // The setup inputs registered adapters declare (their config roots), so the
    // child checks hooks where this node set them up; never session or pane
    // variables, which no adapter declares as setup input.
    // The mod-delivery switch is a per-session override, not a config root.
    let setup: Vec<&'static str> = crate::harness::registry::builtins()
        .registrations()
        .iter()
        .flat_map(|r| r.setup_environment_inputs().iter().copied())
        .filter(|key| *key != crate::protocol::watch::MOD_DELIVERY_ENV)
        .collect();
    vars.filter(|(key, _)| {
        let Some(key) = key.to_str() else {
            return false;
        };
        KEPT_ENV.contains(&key)
            || setup.contains(&key)
            || key.starts_with("LC_")
            || (cfg!(feature = "test-support")
                && (key.starts_with("HT_TEST_") || key == "HT_LEAK_RUN_ID"))
    })
    .collect()
}

/// Requests whose decision or worker is running here: request id -> the
/// requesting node. The per-node running cap counts these, so a lost row
/// (its worker gone) never holds a slot.
pub type Live = Arc<std::sync::Mutex<std::collections::BTreeMap<String, String>>>;

/// Why a slot could not be claimed.
#[derive(Debug, PartialEq, Eq)]
enum Claim {
    /// A decision or worker for this id is already live.
    InFlight,
    /// The requesting node already has this many launches running here.
    Full(u64),
}

/// Claims request `id`'s slot for node `node` in one hold of the live lock:
/// the cap check and the claim are one step, so concurrent requests from one
/// node can never all pass the cap, and a lost row (no live entry) holds no
/// slot. The returned guard frees the slot when dropped.
fn claim_slot(live: &Live, id: &str, node: &str) -> Result<LiveGuard, Claim> {
    let mut map = live.lock().unwrap();
    if map.contains_key(id) {
        return Err(Claim::InFlight);
    }
    let running = map.values().filter(|n| *n == node).count() as u64;
    if running >= MAX_RUNNING_PER_PEER {
        return Err(Claim::Full(running));
    }
    map.insert(id.to_owned(), node.to_owned());
    Ok(LiveGuard {
        live: Arc::clone(live),
        id: id.to_owned(),
    })
}

/// A boxed thread body.
type ThreadBody = Box<dyn FnOnce() + Send>;

/// Spawns `body` on a new thread named for peer launches.
fn spawn_named(body: ThreadBody) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("peer-launch".into())
        .spawn(body)
        .map(|_| ())
}

/// Runs `run(value)` on a thread from `spawn`. The value reaches the thread
/// over a channel only after the thread exists, so when `spawn` fails the
/// value comes back intact (with any guard it holds still alive).
fn hand_to_thread<T: Send + 'static>(
    value: T,
    spawn: impl FnOnce(ThreadBody) -> std::io::Result<()>,
    run: impl FnOnce(T) + Send + 'static,
) -> Result<(), (T, std::io::Error)> {
    let (hand, take) = std::sync::mpsc::channel::<T>();
    if let Err(error) = spawn(Box::new(move || {
        if let Ok(value) = take.recv() {
            run(value);
        }
    })) {
        return Err((value, error));
    }
    // The thread waits for it; a send fails only if that thread is gone.
    let _ = hand.send(value);
    Ok(())
}

/// Removes a request from [`Live`] when its worker ends, a panic included.
struct LiveGuard {
    live: Live,
    id: String,
}

impl Drop for LiveGuard {
    fn drop(&mut self) {
        if let Ok(mut live) = self.live.lock() {
            live.remove(&self.id);
        }
    }
}

pub struct PeerLaunchService {
    /// Requests whose worker thread is running here (by id).
    pub live: Live,
    pub store: Arc<SqliteStore>,
    pub sessions: Arc<HostSessions>,
    pub writer: Arc<FairWriter>,
    pub clock: Arc<dyn Clock>,
    /// This node's `--state-dir`, passed to the children.
    pub state_dir: PathBuf,
    pub log: Arc<dyn Fn(&str) + Send + Sync>,
}

/// The dialled hosts a peer launch may place on: those that can start a guarded
/// native launch (07 §5.3). The built-in host is never dialled and never listed.
pub(crate) fn creation_hosts(sessions: &HostSessions) -> Vec<Arc<HostSession>> {
    sessions
        .attached()
        .into_iter()
        .filter(|session| {
            session.port().native_launch_capability()
                == crate::ports::NativeLaunchCapability::HostGuardedStart
        })
        .collect()
}

/// The dialled hosts a new tab or terminal may be created on: the guarded-start Herdr
/// servers and the herdr-core servers (their creation is `CreateTargetPort::create_target`).
pub(crate) fn target_hosts(sessions: &HostSessions) -> Vec<Arc<HostSession>> {
    let mut hosts = creation_hosts(sessions);
    hosts.extend(
        sessions
            .attached()
            .into_iter()
            .filter(|session| session.port().creates_targets_itself()),
    );
    hosts
}

/// Where one accepted request launches.
pub(crate) struct Placement {
    pub(crate) session: Arc<HostSession>,
    /// The pane for `Pane`, the workspace for `NewTab`.
    pub(crate) target: String,
}

fn now_ms(clock: &dyn Clock) -> i64 {
    clock.utc_now().0
}

fn refused(detail: impl Into<String>) -> LaunchState {
    LaunchState::Refused {
        detail: detail.into(),
    }
}

fn short(id: &str) -> &str {
    &id[..id.len().min(8)]
}

/// A handoff plan's refusal: the sender's message does not verify against its node.
const HANDOFF_UNVERIFIED: &str = "handoff message does not verify against its sender node";
/// The sender node's key is not replicated here yet (unrecorded; a retry decides afresh).
const SENDER_NODE_UNKNOWN: &str = "sender node key not replicated here yet";
/// The sender seat is not replicated here yet (unrecorded).
const SENDER_SEAT_UNKNOWN: &str = "sender seat not replicated here yet";
/// The sender seat is replicated with another home.
const SENDER_SEAT_FOREIGN: &str = "the sender seat is not that node's seat here";
/// The sender seat is replicated here as retired.
const SENDER_SEAT_RETIRED: &str = "the sender seat is retired here";
/// The plan's profile directory is not usable here.
const PROFILE_DIR_UNUSABLE: &str = "profile directory is not an existing absolute directory here";
/// The selected adapter takes no handoff startup input.
const NO_STARTUP_INPUT: &str = "selected adapter has no handoff startup input";

/// What `decide` freezes for an accepted handoff plan.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct HandoffDecision {
    /// The first user message, rendered once (spec section 6) and stored with the row.
    pub(crate) prompt: String,
}

/// The profile directory of a handoff plan as the one frozen [`LaunchDirectoryContext`], through
/// the adapter's `explicit_directory_env`: `None` for a plan without `profile_dir`. The worker
/// repeats this call on the stored plan, so decide and launch agree. `Err` is the refusal detail.
/// A pane plan never reaches this with a directory (`plan_refusal` refuses a pane carrying a
/// `profile_dir`), so freezing `/` for a plan with no `cwd` is only a defensive default.
pub(crate) fn handoff_directory(
    plan: &crate::sync::frame::LaunchPlan,
) -> Result<Option<crate::protocol::handoff::LaunchDirectoryContext>, String> {
    let Some(dir) = plan.profile_dir.as_deref() else {
        return Ok(None);
    };
    let path = Path::new(dir);
    if !path.is_absolute() || !path.is_dir() {
        return Err(PROFILE_DIR_UNUSABLE.into());
    }
    let registration = plan_registration(plan)?;
    let env = registration
        .launch_policy()
        .expect("launch_registration checked the policy")
        .explicit_directory_env(path)
        .map_err(|error| clamp_detail(&error.detail))?;
    let context = crate::protocol::handoff::LaunchDirectoryContext {
        profile_dir: path.to_path_buf(),
        cwd: plan
            .cwd
            .as_deref()
            .map_or_else(|| "/".into(), PathBuf::from),
        env,
    };
    context
        .validate()
        .map_err(|_| PROFILE_DIR_UNUSABLE.to_owned())?;
    Ok(Some(context))
}

/// The registration of the plan's harness, with a launch policy.
fn plan_registration(
    plan: &crate::sync::frame::LaunchPlan,
) -> Result<&'static crate::harness::registry::Registration, String> {
    let registry = crate::harness::registry::builtins();
    let id = registry.agent(&plan.kind).map_err(|_| {
        format!(
            "harness {} is not registered here",
            clamp_detail(&plan.kind)
        )
    })?;
    crate::harness::launch::launch_registration(
        registry,
        crate::harness::registry::OccupantHarness::Agent(id),
    )
    .map_err(|error| clamp_detail(&error.detail))
}

/// Whether the adapter takes the handoff startup input for `argv`; the same
/// `prepare_startup_input` call as the local handoff's `select_startup_input`. `Err` is the
/// refusal detail (an adapter error passes through).
fn check_startup_input(
    policy: &dyn crate::harness::adapter::LaunchPolicy,
    argv: &[String],
) -> Result<(), String> {
    startup_input_verdict(policy.prepare_startup_input(
        argv,
        &crate::harness::adapter::StartupInputSpec {
            max_text_bytes: crate::ports::NativeLaunchRequest::MAX_ARG_BYTES,
        },
    ))
}

/// The refusal detail of an adapter's startup-input answer, if it is one.
fn startup_input_verdict<T>(
    answer: Result<Option<T>, crate::protocol::results::ApiError>,
) -> Result<(), String> {
    match answer {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(NO_STARTUP_INPUT.into()),
        Err(error) => Err(clamp_detail(&error.detail)),
    }
}

/// What this node's store says about a handoff's sender.
struct SenderView {
    /// The public keys of the sender node's logs, newest incarnation first.
    keys: Vec<[u8; 32]>,
    /// The sender seat's directory row: home node, whether it is local, replicated name,
    /// whether it is retired.
    seat: Option<(String, bool, Option<String>, bool)>,
}

impl PeerLaunchService {
    fn budget(&self) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(self.clock.monotonic_now().0 + CALL_BUDGET_MS),
            cancellation: Cancellation::default(),
        }
    }

    fn write<R>(
        &self,
        write: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<R>,
    ) -> Result<R, ApiError> {
        let budget = self.budget();
        let _turn = self.writer.enter_background(&budget, self.clock.as_ref())?;
        self.store.sync_operator_quantum(&budget, |tx, _, _| {
            write(tx).map_err(crate::store::connection::store_error)
        })
    }

    fn read<R>(
        &self,
        read: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<R>,
    ) -> Result<R, ApiError> {
        self.store.sync_read(&self.budget(), |conn, _| {
            read(conn).map_err(crate::store::connection::store_error)
        })
    }

    /// At daemon start: requests left running by the previous daemon are
    /// failed (never resumed), old rows deleted.
    pub fn settle_at_start(&self) {
        let now = now_ms(self.clock.as_ref());
        match self.write(|conn| peer_requests::settle_at_start(conn, now)) {
            Ok(ids) => {
                for id in ids {
                    (self.log)(&format!(
                        "peer launch {}: failed: interrupted: the daemon restarted",
                        short(&id)
                    ));
                }
            }
            Err(error) => (self.log)(&format!(
                "peer launch: settling interrupted requests failed: {}",
                error.detail
            )),
        }
    }

    /// The one attached host and pane (or workspace) the plan names.
    pub(crate) fn place(&self, target: &LaunchTarget) -> Result<Placement, LaunchState> {
        let budget = self.budget();
        let mut found: Vec<Placement> = Vec::new();
        let mut failed = false;
        let new_target = !matches!(target, LaunchTarget::Pane { .. });
        let sessions = if new_target {
            target_hosts(&self.sessions)
        } else {
            creation_hosts(&self.sessions)
        };
        if let LaunchTarget::NewTerminal { .. } = target {
            // A terminal lives on a herdr-core, which has no panes to read.
            let mut cores: Vec<_> = sessions
                .into_iter()
                .filter(|s| s.port().creates_targets_itself())
                .collect();
            return match cores.len() {
                1 => Ok(Placement {
                    session: cores.remove(0),
                    target: String::new(),
                }),
                0 => Err(refused("no herdr-core is attached here")),
                n => Err(refused(format!("ambiguous herdr-core: {n} attached here"))),
            };
        }
        if sessions.is_empty() {
            return Err(LaunchState::Unavailable {
                reason: "no Herdr server is attached here".into(),
            });
        }
        for session in sessions {
            let Ok(topology) = session.port().live_topology(&budget) else {
                failed = true;
                continue;
            };
            match target {
                LaunchTarget::Pane { space, tab, pane } => {
                    let outcome = address::resolve_parts(
                        &topology,
                        space.as_deref(),
                        tab.as_deref(),
                        Some(pane),
                    );
                    let panes = match outcome {
                        address::Outcome::Resolved(c) => vec![c],
                        address::Outcome::Ambiguous(cs) => cs,
                        _ => Vec::new(),
                    };
                    found.extend(panes.into_iter().map(|c| Placement {
                        session: Arc::clone(&session),
                        target: c.pane.as_str().to_owned(),
                    }));
                }
                LaunchTarget::NewTab { space, .. } => {
                    found.extend(
                        topology
                            .spaces
                            .iter()
                            .filter(|s| &s.id == space || s.label.as_deref() == Some(space))
                            .map(|s| Placement {
                                session: Arc::clone(&session),
                                target: s.id.clone(),
                            }),
                    );
                }
                LaunchTarget::NewTerminal { .. } | LaunchTarget::Unknown => {}
            }
        }
        let what = match target {
            LaunchTarget::NewTab { .. } => "workspace",
            _ => "pane",
        };
        if failed && found.len() < 2 {
            return Err(LaunchState::Unavailable {
                reason: "a Herdr server here could not be read".into(),
            });
        }
        match found.len() {
            1 => Ok(found.remove(0)),
            0 => Err(refused(format!("no such {what} here"))),
            n => Err(refused(format!("ambiguous {what}: {n} match here"))),
        }
    }

    /// Whether a pane is held for recovery here (C2): a held target refuses.
    fn held(&self, placement: &Placement) -> Result<bool, ApiError> {
        let host = placement.session.host_id().get();
        let pane = placement.target.clone();
        self.read(move |conn| {
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM recovery_holds WHERE host_id=?1 AND target_id=?2 \
                 AND released_at IS NULL)",
                rusqlite::params![host, pane],
                |r| r.get(0),
            )
        })
    }

    fn requester_tag(&self, from: NodeUuid) -> String {
        self.read(move |conn| crate::store::node_directory::get(conn, from))
            .ok()
            .flatten()
            .map_or_else(
                || from.to_string()[..8].to_owned(),
                |row| row.tag.to_string(),
            )
    }

    /// Records `row` refused with `detail` (a repeat answers the same) and
    /// logs it.
    fn refuse(&self, from: NodeUuid, row: &PeerRequest, detail: &str) -> LaunchState {
        let detail = clamp_detail(detail);
        let now = now_ms(self.clock.as_ref());
        let recorded = self.write(|conn| {
            let (stored, inserted) = peer_requests::insert_or_get(conn, row)?;
            if inserted {
                peer_requests::record(conn, &row.id, "refused", None, None, Some(&detail), now)?;
                return Ok(None);
            }
            // A concurrent decision for the same id got there first: its answer.
            Ok(Some(stored))
        });
        if let Ok(Some(stored)) = recorded {
            return stored.answer();
        }
        (self.log)(&format!(
            "peer launch {} from {}: refused: {detail}{}",
            short(&row.id),
            self.requester_tag(from),
            if recorded.is_err() {
                " (not recorded)"
            } else {
                ""
            }
        ));
        refused(detail)
    }

    /// A `running` row whose worker is gone: recorded failed and unconfirmed
    /// (its outcome write failed; the launch may have happened).
    fn lost(&self, row: &PeerRequest) -> LaunchState {
        // A new tab's pane may be unknown: its workspace is named instead.
        let place = match &row.pane {
            Some(pane) => format!("pane {pane}"),
            None => serde_json::from_str::<crate::sync::frame::LaunchPlan>(&row.plan)
                .ok()
                .and_then(|plan| match plan.target {
                    LaunchTarget::NewTab { space, .. } => Some(format!("workspace {space}")),
                    _ => None,
                })
                .unwrap_or_else(|| "its target".to_owned()),
        };
        let detail = format!(
            "{}: the launch's outcome was lost; inspect {place} here before launching again",
            peer_requests::UNCONFIRMED
        );
        let now = now_ms(self.clock.as_ref());
        let written = self.write(|conn| {
            if peer_requests::record(conn, &row.id, "failed", None, None, Some(&detail), now)? {
                return Ok(None);
            }
            // Settled meanwhile (its worker's write landed after all): that answer.
            peer_requests::get(conn, &row.id)
        });
        if let Ok(Some(stored)) = written {
            return stored.answer();
        }
        PeerRequest {
            state: "failed".into(),
            detail: Some(detail),
            ..row.clone()
        }
        .answer()
    }

    /// The handoff checks of `decide`, after placement, cwd and hold: the sender's signed block
    /// against its node, its seat against the directory, the profile directory and the
    /// harness's startup input. `Ok(None)` for a plan that is not a handoff. `Err(Refused)` is
    /// to be recorded; `Err(Unavailable)` (the sender's node or seat not replicated yet) is
    /// answered unrecorded, so a retry decides afresh. Writes nothing.
    pub(crate) fn handoff_refusal(
        &self,
        from: NodeUuid,
        request: &LaunchRequest,
    ) -> Result<Option<HandoffDecision>, LaunchState> {
        let plan = &request.plan;
        let Some(block) = plan.handoff() else {
            return Ok(None);
        };
        if block.sender_node != Some(from)
            || block.author.as_ref().map(|a| a.as_str()) != Some(request.requester_seat.as_str())
            || block.request != request.id
        {
            return Err(refused(HANDOFF_UNVERIFIED));
        }
        let seat = request.requester_seat.clone();
        let view = self
            .read(move |conn| {
                use rusqlite::OptionalExtension;
                let keys = crate::store::materialize::node_keys(conn, from)?;
                let seat = conn
                    .query_row(
                        "SELECT home_node,is_local,name,state='retired' FROM seats WHERE id=?1",
                        [&seat],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                    )
                    .optional()?;
                Ok(SenderView { keys, seat })
            })
            .map_err(|error| LaunchState::Unavailable {
                reason: format!("store: {}", error.detail),
            })?;
        if view.keys.is_empty() {
            return Err(LaunchState::Unavailable {
                reason: SENDER_NODE_UNKNOWN.into(),
            });
        }
        if !view.keys.iter().any(|key| block.verify(key).is_ok()) {
            return Err(refused(HANDOFF_UNVERIFIED));
        }
        let Some((home, is_local, name, retired)) = view.seat else {
            return Err(LaunchState::Unavailable {
                reason: SENDER_SEAT_UNKNOWN.into(),
            });
        };
        if is_local || home != from.to_string() {
            return Err(refused(SENDER_SEAT_FOREIGN));
        }
        if retired {
            return Err(refused(SENDER_SEAT_RETIRED));
        }
        handoff_directory(plan).map_err(refused)?;
        let registration = plan_registration(plan).map_err(refused)?;
        check_startup_input(
            registration
                .launch_policy()
                .expect("launch_registration checked the policy"),
            &plan.argv,
        )
        .map_err(refused)?;
        let prompt = crate::cli::handoff::summary_prompt::render_v2(
            plan.handoff_prompt.as_deref(),
            &crate::cli::handoff::summary_prompt::SenderLine {
                name,
                id: request.requester_seat.clone(),
            },
        );
        Ok(Some(HandoffDecision { prompt }))
    }

    fn decide(&self, from: NodeUuid, request: &LaunchRequest) -> Result<LaunchState, ApiError> {
        let id = request.id.to_string();
        let lookup = id.clone();
        if let Some(row) = self.read(move |conn| peer_requests::get(conn, &lookup))? {
            if row.node_uuid != from.to_string() {
                (self.log)(&format!(
                    "peer launch {} from {}: refused: request id in use by another node",
                    short(&id),
                    self.requester_tag(from)
                ));
                return Ok(refused("request id in use by another node"));
            }
            if row.requester_seat.as_deref().unwrap_or_default()
                != clamp_detail(&request.requester_seat)
            {
                return Ok(refused("request id in use by another seat"));
            }
            // A row still `running` whose worker is gone (its outcome could not be
            // written): the launch may have happened; never resumed, never relaunched.
            if row.state == "running" && !self.live.lock().unwrap().contains_key(&id) {
                return Ok(self.lost(&row));
            }
            return Ok(row.answer());
        }
        let plan = &request.plan;
        let new_tab = matches!(
            plan.target,
            LaunchTarget::NewTab { .. } | LaunchTarget::NewTerminal { .. }
        );
        let plan_text = serde_json::to_string(plan).unwrap_or_default();
        let row = PeerRequest {
            id: id.clone(),
            node_uuid: from.to_string(),
            requester_seat: (!request.requester_seat.is_empty())
                .then(|| clamp_detail(&request.requester_seat)),
            // The table's CHECK knows two kinds; a terminal is a new target like a tab.
            kind: if new_tab { "new_tab" } else { "pane" }.into(),
            plan: plan_text,
            state: "running".into(),
            pane: None,
            seat: None,
            detail: None,
            handoff_thread: None,
            handoff_message: None,
            handoff_prompt: None,
            created_at: now_ms(self.clock.as_ref()),
            updated_at: 0,
        };
        if let Some(why) = plan_refusal(plan, row.plan.len()) {
            return Ok(self.refuse(from, &row, &why));
        }
        let placement = match self.place(&plan.target) {
            Ok(placement) => placement,
            // A refusal is recorded (a repeat answers it); an unavailable
            // answer is not, so a repeat decides again.
            Err(LaunchState::Refused { detail }) => return Ok(self.refuse(from, &row, &detail)),
            Err(state) => return Ok(state),
        };
        if new_tab {
            let cwd = plan.cwd.as_deref().map(Path::new);
            if !cwd.is_some_and(|cwd| cwd.is_absolute() && cwd.is_dir()) {
                return Ok(self.refuse(
                    from,
                    &row,
                    "a new tab needs an existing absolute cwd here",
                ));
            }
        } else if self.held(&placement)? {
            return Ok(self.refuse(from, &row, "that pane is held for recovery here"));
        }
        // The handoff checks run after the existing ones and before the slot claim; a
        // refusal is recorded (no fact is written), an unavailable answer is not.
        let handoff_prompt = match self.handoff_refusal(from, request) {
            Ok(decision) => decision.map(|d| d.prompt),
            Err(LaunchState::Refused { detail }) => return Ok(self.refuse(from, &row, &detail)),
            Err(state) => return Ok(state),
        };
        // A pane launch names its pane from the start, so an interruption at any
        // point reads as unconfirmed (the launch child may have started).
        let row = PeerRequest {
            pane: (!new_tab).then(|| placement.target.clone()),
            handoff_prompt,
            ..row
        };
        // The cap and the claim on a slot are one step under the live lock (no
        // two decisions both pass it), and the claim precedes the row's commit
        // (a repeat never reads a fresh `running` row as one whose worker is gone).
        let from_text = from.to_string();
        let slot = match claim_slot(&self.live, &id, &from_text) {
            Ok(slot) => slot,
            Err(Claim::InFlight) => {
                // Another decision for this id is in flight: it answers.
                return Ok(LaunchState::Running {
                    pane: row.pane.clone(),
                });
            }
            Err(Claim::Full(running)) => {
                return Ok(LaunchState::Unavailable {
                    reason: format!("{running} launches for that machine are already running here"),
                });
            }
        };
        let gated = placement.session.port().creates_targets_itself();
        let (stored, inserted) = self.write(|conn| {
            let (stored, inserted) = peer_requests::insert_or_get(conn, &row)?;
            // The gate-only record exists before the terminal does: its shim waits for the
            // creation instead of being refused as unknown.
            if inserted && gated {
                peer_requests::open_gate(conn, &row.id)?;
            }
            Ok((stored, inserted))
        })?;
        if !inserted {
            return Ok(stored.answer());
        }
        let tag = self.requester_tag(from);
        (self.log)(&format!(
            "peer launch {} from {tag}: accepted ({}, {} on {})",
            short(&id),
            row.kind,
            plan.kind,
            placement.target
        ));
        let pane = (!new_tab).then(|| placement.target.clone());
        let worker = Worker {
            slot,
            id,
            tag,
            store: Arc::clone(&self.store),
            writer: Arc::clone(&self.writer),
            clock: Arc::clone(&self.clock),
            state_dir: self.state_dir.clone(),
            session: Arc::clone(&placement.session),
            log: Arc::clone(&self.log),
        };
        let request = request.clone();
        let target = placement.target;
        // If the thread cannot be created, this side still holds the worker, and
        // with it the request's slot until the failed row is written (a repeat
        // never reads it as lost).
        let started = hand_to_thread(worker, spawn_named, move |worker: Worker| {
            worker.run(&request, target)
        });
        if let Err((worker, error)) = started {
            // Nothing started: a plain failure, never an unconfirmed one.
            let detail = clamp_detail(&format!("could not start the launch worker: {error}"));
            worker.record("failed", None, None, Some(&detail));
            drop(worker);
            return Ok(LaunchState::Failed {
                detail,
                pane: None,
                unconfirmed: false,
                staged: None,
            });
        }
        Ok(LaunchState::Running { pane })
    }
}

impl PeerLauncher for PeerLaunchService {
    fn launch(&self, from: NodeUuid, request: LaunchRequest) -> LaunchState {
        self.decide(from, &request)
            .unwrap_or_else(|error| LaunchState::Unavailable {
                reason: format!("store: {}", error.detail),
            })
    }
}

/// Why `plan` (serialized to `plan_bytes`) is refused before anything runs
/// here, if it is: too large for the table, a harness this node does not
/// register, or a tab label that starts like a flag.
pub fn plan_refusal(plan: &crate::sync::frame::LaunchPlan, plan_bytes: usize) -> Option<String> {
    if plan_bytes > peer_requests::MAX_PLAN_BYTES {
        return Some("launch plan too large".into());
    }
    if crate::harness::registry::builtins()
        .agent(&plan.kind)
        .is_err()
    {
        return Some(format!(
            "harness {} is not registered here",
            clamp_detail(&plan.kind)
        ));
    }
    if let LaunchTarget::NewTab { label, .. } | LaunchTarget::NewTerminal { label } = &plan.target
        && label.trim_start().starts_with('-')
    {
        return Some("a tab label must not start with -".into());
    }
    if matches!(plan.target, LaunchTarget::Pane { .. }) && plan.profile_dir.is_some() {
        return Some(PANE_PROFILE_DIR.into());
    }
    None
}

/// The refusal of a pane plan carrying a profile directory (49 bytes): Herdr's `agent.start`
/// carries argv only, so a pane cannot take the directory's environment.
const PANE_PROFILE_DIR: &str = "a pane launch cannot change the profile directory";

/// The `internal peer-target-create` arguments of a new tab (`workspace` names its
/// workspace) or, with no workspace, a new terminal: its label ends in `[from <tag>]`, the
/// requesting node. `gate_argv` is what a herdr-core terminal starts. `directory_context` is a
/// handoff tab's frozen profile directory context (`None` for a bare plan and for hc): its env
/// goes to the created tab, as a local tab's `CreateTabRequest.env` does.
pub fn target_args(
    workspace: Option<&str>,
    label: &str,
    tag: &str,
    cwd: &str,
    gate_argv: &[String],
    directory_context: Option<&crate::protocol::handoff::LaunchDirectoryContext>,
) -> Vec<String> {
    // `--flag=value`: a value can never be read as another flag.
    let mut args = vec!["internal".into(), "peer-target-create".into()];
    match workspace {
        Some(workspace) => args.push(format!("--workspace={workspace}")),
        None => args.push("--new-terminal".into()),
    }
    args.push(format!("--label={} [from {tag}]", label.trim()));
    args.push(format!("--cwd={cwd}"));
    if let Some(context) = directory_context {
        args.push(format!(
            "--directory-context={}",
            serde_json::to_string(context).expect("a directory context serializes")
        ));
    }
    if !gate_argv.is_empty() {
        args.push("--".into());
        args.extend(gate_argv.iter().cloned());
    }
    args
}

/// What a herdr-core terminal of a peer launch starts:
/// `<program> --state-dir <dir> internal launch-gate --bootstrap <request>:1 -- <executable>
/// <host args> <plan.argv>`. The executable is resolved on `path` from the plan's kind (the
/// plan carries arguments only); the core hands a terminal none of the caller's routing, so the
/// state directory rides the argv. `config_root` is the launching caller's config root, pushed
/// as `--config-root <harness>=<dir>`. A bare peer plan carries no root (`None`): the gate's
/// scrub leaves the harness's default root and the core's ambient root cannot win. A handoff
/// plan passes its frozen profile directory here and its frozen `prompt`, which the adapter's
/// startup-input template places in the plan's argv (the same call as `decide`'s check).
pub fn gate_argv(
    program: &Path,
    state_dir: &Path,
    request: &str,
    plan: &crate::sync::frame::LaunchPlan,
    path: Option<&std::ffi::OsStr>,
    config_root: Option<&Path>,
    prompt: Option<&str>,
) -> Result<Vec<String>, String> {
    let text = |p: &Path| {
        p.to_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("path {} is not UTF-8", p.display()))
    };
    let registry = crate::harness::registry::builtins();
    let id = registry.agent(&plan.kind).map_err(|_| {
        format!(
            "harness {} is not registered here",
            clamp_detail(&plan.kind)
        )
    })?;
    let registration = registry.by_id(id).map_err(|_| {
        format!(
            "harness {} is not registered here",
            clamp_detail(&plan.kind)
        )
    })?;
    let crate::harness::adapter::ExecutableLookup::Path(name) = registration.metadata().executable
    else {
        return Err(format!("harness {} has no executable", plan.kind));
    };
    let executable = crate::cli::hook::resolve_on_path(name, path)
        .ok_or_else(|| format!("no executable `{name}` on PATH"))?;
    let mut harness_argv = registration.host_launch_argv(crate::ports::HostKind::Hc);
    match prompt {
        None => harness_argv.extend(plan.argv.iter().cloned()),
        Some(prompt) => {
            let template = registration
                .launch_policy()
                .ok_or_else(|| NO_STARTUP_INPUT.to_owned())?
                .prepare_startup_input(
                    &plan.argv,
                    &crate::harness::adapter::StartupInputSpec {
                        max_text_bytes: crate::ports::NativeLaunchRequest::MAX_ARG_BYTES,
                    },
                )
                .map_err(|e| clamp_detail(&e.detail))?
                .ok_or_else(|| NO_STARTUP_INPUT.to_owned())?;
            let (applied, _) = template
                .apply(&plan.argv, prompt)
                .map_err(|e| clamp_detail(&e.detail))?;
            harness_argv.extend(applied);
        }
    }
    crate::ports::NativeLaunchRequest::validate_argv(
        &crate::harness::launch::compose_native_argv(
            crate::harness::registry::OccupantHarness::Agent(id),
            harness_argv.clone(),
            vec![],
        )
        .map_err(|e| e.detail)?,
    )
    .map_err(|e| e.to_string())?;
    let mut argv = vec![
        text(program)?,
        "--state-dir".into(),
        text(state_dir)?,
        "internal".into(),
        "launch-gate".into(),
        "--bootstrap".into(),
        format!("{request}:{}", peer_requests::GATE_ATTEMPT),
    ];
    if let Some(root) = config_root {
        argv.push("--config-root".into());
        argv.push(format!("{}={}", registration.metadata().id, text(root)?));
    }
    argv.push("--".into());
    argv.push(text(&executable)?);
    argv.extend(harness_argv);
    Ok(argv)
}

/// The ordinary `launch --pane` arguments of `plan` on `pane`.
pub fn launch_args(pane: &str, plan: &crate::sync::frame::LaunchPlan) -> Vec<String> {
    let mut args = vec![
        "launch".to_owned(),
        format!("--pane={pane}"),
        format!("--kind={}", plan.kind),
    ];
    if !plan.argv.is_empty() {
        args.push("--".into());
        args.extend(plan.argv.iter().cloned());
    }
    args
}

/// The `internal peer-handoff-launch` arguments of request `request` on `pane`: the child stages
/// the request's thread and message, then launches (spec section 3.2). `--flag=value`, as above.
pub fn handoff_launch_args(pane: &str, request: &str) -> Vec<String> {
    vec![
        "internal".into(),
        "peer-handoff-launch".into(),
        format!("--request={request}"),
        format!("--pane={pane}"),
    ]
}

/// The child that launches `plan` on `pane`: the staging child for a handoff plan, plain
/// `launch --pane` for every other.
fn launch_child_args(
    pane: &str,
    request: &str,
    plan: &crate::sync::frame::LaunchPlan,
) -> Vec<String> {
    match plan.handoff() {
        Some(_) => handoff_launch_args(pane, request),
        None => launch_args(pane, plan),
    }
}

/// Runs one accepted request to its outcome.
struct Worker {
    /// Frees the request's live slot when the worker ends, a panic included.
    slot: LiveGuard,
    id: String,
    tag: String,
    store: Arc<SqliteStore>,
    writer: Arc<FairWriter>,
    clock: Arc<dyn Clock>,
    state_dir: PathBuf,
    /// The host the target is created on. A herdr-core (`creates_targets_itself`) starts the
    /// gate argv itself in its terminals.
    session: Arc<HostSession>,
    log: Arc<dyn Fn(&str) + Send + Sync>,
}

/// One finished child: exit success, stdout, the last stderr line.
struct ChildOutput {
    ok: bool,
    /// The child process started (`spawn` succeeded). A child that never
    /// started cannot have launched anything.
    spawned: bool,
    /// The exit status, when the child exited by itself.
    code: Option<i32>,
    stdout: String,
    error: String,
}

impl ChildOutput {
    /// Started, then killed (timeout, signal) before it answered: it may have
    /// done its work.
    fn may_have_started(&self) -> bool {
        self.spawned && self.code.is_none()
    }
}

/// The failure detail of a launch child that did not report a seat, and
/// whether the start may have happened (exit 5, or killed after it started).
fn launch_failure(out: &ChildOutput, pane: &str) -> (String, bool) {
    if out.code == Some(crate::cli::launch::EXIT_START_UNCONFIRMED) || out.may_have_started() {
        return (
            format!(
                "{}: inspect pane {pane} here before launching again",
                peer_requests::UNCONFIRMED
            ),
            true,
        );
    }
    let detail = if out.error.is_empty() {
        "launch did not report a seat".to_owned()
    } else {
        format!("launch: {}", out.error)
    };
    (detail, false)
}

/// Runs `command` (stdout and stderr piped) to its end, killing it past
/// [`CHILD_TIMEOUT`].
fn run_child(mut command: Command) -> ChildOutput {
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return ChildOutput {
                ok: false,
                spawned: false,
                code: None,
                stdout: String::new(),
                error: format!("could not start: {error}"),
            };
        }
    };
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < CHILD_TIMEOUT => std::thread::sleep(CHILD_POLL),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let stdout = String::from_utf8_lossy(&stdout.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&stderr.join().unwrap_or_default()).into_owned();
    let last = stderr
        .lines()
        .rfind(|l| !l.trim().is_empty())
        .unwrap_or_default()
        .to_owned();
    match status {
        Some(status) => ChildOutput {
            ok: status.success(),
            spawned: true,
            code: status.code(),
            stdout,
            error: last,
        },
        None => ChildOutput {
            ok: false,
            spawned: true,
            code: None,
            stdout,
            error: "timed out".into(),
        },
    }
}

/// Longest detail recorded in `peer_requests.detail` (its CHECK), in bytes.
const MAX_DETAIL_BYTES: usize = 1024;

/// `text` cut to fit `peer_requests.detail`, on a char boundary.
fn clamp_detail(text: &str) -> String {
    let mut end = text.len().min(MAX_DETAIL_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Reads all of `pipe` on its own thread, so a child that writes more than
/// the pipe buffer never blocks on it.
fn drain<R: std::io::Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut out = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = std::io::Read::read_to_end(&mut pipe, &mut out);
        }
        out
    })
}

impl Worker {
    fn budget(&self) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(self.clock.monotonic_now().0 + CALL_BUDGET_MS),
            cancellation: Cancellation::default(),
        }
    }

    /// The prompt `decide` froze on the request's row.
    fn frozen_prompt(&self) -> Result<Option<String>, ApiError> {
        self.store.sync_read(&self.budget(), |conn, _| {
            peer_requests::get(conn, &self.id)
                .map(|row| row.and_then(|row| row.handoff_prompt))
                .map_err(crate::store::connection::store_error)
        })
    }

    fn record(&self, state: &str, pane: Option<&str>, seat: Option<&str>, detail: Option<&str>) {
        self.retrying(&format!("recording {state}"), |conn, now| {
            peer_requests::record(conn, &self.id, state, pane, seat, detail, now)
        });
    }

    /// Runs `write` (one store write at the current time), retried on [`RECORD_RETRIES`].
    /// The result says whether it landed.
    fn retrying(
        &self,
        what: &str,
        write: impl Fn(&rusqlite::Connection, i64) -> rusqlite::Result<bool>,
    ) -> bool {
        let attempt = || {
            let now = now_ms(self.clock.as_ref());
            let budget = CallBudget {
                deadline: MonoInstant(self.clock.monotonic_now().0 + CALL_BUDGET_MS),
                cancellation: Cancellation::default(),
            };
            self.writer
                .enter_background(&budget, self.clock.as_ref())
                .and_then(|_turn| {
                    self.store.sync_operator_quantum(&budget, |tx, _, _| {
                        write(tx, now).map_err(crate::store::connection::store_error)
                    })
                })
        };
        let mut written = attempt();
        for delay in RECORD_RETRIES {
            if written.is_ok() {
                break;
            }
            std::thread::sleep(delay);
            written = attempt();
        }
        // Still unwritten: the row stays `running` and, once this worker is gone, a
        // repeat answers it unconfirmed (`PeerLaunchService::lost`).
        match written {
            Ok(changed) => changed,
            Err(error) => {
                (self.log)(&format!(
                    "peer launch {}: {what} failed: {}",
                    short(&self.id),
                    error.detail
                ));
                false
            }
        }
    }

    /// The seat of a gate-only launch, allocated before its gate opens (see
    /// `launch_gate::allocate_seat`).
    fn allocate_gate_seat(
        &self,
        plan: &crate::sync::frame::LaunchPlan,
        created: &crate::ports::CreatedTarget,
    ) -> Result<crate::protocol::ids::SeatId, ApiError> {
        use crate::protocol::results::ErrorCode;
        let registry = crate::harness::registry::builtins();
        let harness =
            crate::harness::registry::OccupantHarness::Agent(registry.agent(&plan.kind).map_err(
                |_| ApiError::new(ErrorCode::UnsupportedHarness, "harness not registered"),
            )?);
        // The seat's own budget: nothing above this call bounds it shorter.
        let budget = CallBudget {
            deadline: MonoInstant(
                self.clock.monotonic_now().0 + super::launch_gate::GATE_SEAT_BUDGET_MS,
            ),
            cancellation: Cancellation::default(),
        };
        super::launch_gate::allocate_seat(
            &super::launch_gate::GateSeat {
                session: &self.session,
                store: self.store.as_ref(),
                writer: &self.writer,
                clock: self.clock.as_ref(),
                budget: &budget,
                harness,
                operation: crate::protocol::ids::OperationId::new(format!(
                    "peer-launch:{}",
                    self.id
                )),
            },
            created,
        )
    }

    /// Stages the handoff's facts and records the gate in one commit, retried on
    /// [`RECORD_RETRIES`] (`stage` is idempotent on the request id). The audit line names the
    /// request, thread, message and sender node (spec section 8).
    fn stage_handoff(
        &self,
        seat: &crate::protocol::ids::SeatId,
        created: &str,
        terminal: &str,
    ) -> Result<(), ApiError> {
        let attempt = || {
            let budget = self.budget();
            let _turn = self.writer.enter_background(&budget, self.clock.as_ref())?;
            self.store
                .sync_operator_quantum(&budget, |tx, context, decision| {
                    crate::store::peer_handoff::stage(
                        tx,
                        context,
                        decision,
                        &self.id,
                        seat,
                        Some(crate::store::peer_handoff::GateRecord { created, terminal }),
                    )
                })
        };
        let mut staged = attempt();
        for delay in RECORD_RETRIES {
            if staged.is_ok() {
                break;
            }
            std::thread::sleep(delay);
            staged = attempt();
        }
        let staged = staged?;
        if staged.fresh {
            (self.log)(&format!(
                "peer launch {} from {}: staged message {} in thread {} (sender node {})",
                short(&self.id),
                self.tag,
                staged.message.as_str(),
                staged.thread.as_str(),
                staged.sender_node
            ));
        }
        Ok(())
    }

    /// The program the children run: this binary (a test stand-in in
    /// test-support builds when `HT_TEST_PEER_LAUNCH_STUB` names one).
    fn program() -> PathBuf {
        #[cfg(feature = "test-support")]
        if let Some(stub) = std::env::var_os("HT_TEST_PEER_LAUNCH_STUB") {
            return PathBuf::from(stub);
        }
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("herdr-threads"))
    }

    fn child(&self, args: &[String]) -> ChildOutput {
        let mut command = Command::new(Self::program());
        command
            .env_clear()
            .envs(child_env(std::env::vars_os()))
            .arg("--json")
            .arg("--state-dir")
            .arg(&self.state_dir)
            .arg("--host-endpoint")
            .arg(self.session.locator().as_path())
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // The child ends with this daemon: a request the next daemon fails as
        // interrupted never completes behind its back (Linux; elsewhere it may).
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: prctl is async-signal-safe; nothing else runs between
            // fork and exec here.
            unsafe {
                command.pre_exec(|| {
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        run_child(command)
    }

    fn fail(&self, pane: Option<&str>, detail: &str) {
        let detail = clamp_detail(detail);
        let detail = detail.as_str();
        self.record("failed", pane, None, Some(detail));
        (self.log)(&format!(
            "peer launch {} from {}: failed: {detail}",
            short(&self.id),
            self.tag
        ));
    }

    fn run(self, request: &LaunchRequest, target: String) {
        self.run_to_outcome(request, target);
        drop(self.slot);
    }

    fn run_to_outcome(&self, request: &LaunchRequest, target: String) {
        let plan = &request.plan;
        let pane = match &plan.target {
            LaunchTarget::NewTab { label, .. } | LaunchTarget::NewTerminal { label } => {
                let workspace =
                    matches!(plan.target, LaunchTarget::NewTab { .. }).then_some(target.as_str());
                let hc = self.session.port().creates_targets_itself();
                let handoff = plan.handoff().is_some();
                let gate = if hc {
                    // A handoff plan's gate starts the harness with the frozen prompt and the
                    // frozen profile directory; a bare plan carries neither (the gate's scrub
                    // keeps the core's root out).
                    let (root, prompt) = if handoff {
                        let prompt = match self.frozen_prompt() {
                            Ok(Some(prompt)) => prompt,
                            Ok(None) => return self.fail(None, "gate: no frozen handoff prompt"),
                            Err(error) => {
                                return self.fail(None, &format!("gate: {}", error.detail));
                            }
                        };
                        let root = match handoff_directory(plan) {
                            Ok(context) => context.map(|c| c.profile_dir),
                            Err(why) => return self.fail(None, &format!("gate: {why}")),
                        };
                        (root, Some(prompt))
                    } else {
                        (None, None)
                    };
                    match gate_argv(
                        &Self::program(),
                        &self.state_dir,
                        &self.id,
                        plan,
                        std::env::var_os("PATH").as_deref(),
                        root.as_deref(),
                        prompt.as_deref(),
                    ) {
                        Ok(argv) => argv,
                        Err(why) => return self.fail(None, &format!("gate: {why}")),
                    }
                } else {
                    Vec::new()
                };
                // A Herdr handoff tab takes the frozen profile env at creation (the agent started
                // in its root pane inherits it); hc carries the root in the gate argv instead.
                let context = if handoff && !hc {
                    match handoff_directory(plan) {
                        Ok(context) => context,
                        Err(why) => return self.fail(None, &format!("creation: {why}")),
                    }
                } else {
                    None
                };
                let out = self.child(&target_args(
                    workspace,
                    label,
                    &self.tag,
                    plan.cwd.as_deref().unwrap_or_default(),
                    &gate,
                    context.as_ref(),
                ));
                let value = serde_json::from_str::<serde_json::Value>(out.stdout.trim()).ok();
                let created = value.as_ref().and_then(|v| {
                    v["pane"]
                        .as_str()
                        .or_else(|| v["terminal"].as_str())
                        .map(str::to_owned)
                });
                match (out.ok, created) {
                    // A herdr-core terminal runs the gate itself: this node records its
                    // creation and seat, which opens the gate (N15).
                    (true, Some(terminal)) if hc => {
                        self.record("running", Some(&terminal), None, None);
                        let created = value
                            .as_ref()
                            .and_then(|v| {
                                serde_json::from_value::<crate::ports::CreatedTarget>(
                                    v["created"].clone(),
                                )
                                .ok()
                            })
                            .filter(|c| c.validate().is_ok() && c.terminal.as_str() == terminal);
                        let Some(created) = created else {
                            return self.fail(
                                Some(&terminal),
                                &format!("creation of {terminal} reported no valid record"),
                            );
                        };
                        let seat = match self.allocate_gate_seat(plan, &created) {
                            Ok(seat) => seat,
                            Err(error) => {
                                return self.fail(
                                    Some(&terminal),
                                    &format!("seat for {terminal}: {}", error.detail),
                                );
                            }
                        };
                        let text = serde_json::to_string(&created).unwrap_or_default();
                        let opened = if handoff {
                            // Facts and gate record in one commit: the gate opens only on the
                            // staged Send (`gate_view`). A failure records plain `failed`.
                            match self.stage_handoff(&seat, &text, &terminal) {
                                Ok(()) => true,
                                Err(error) => {
                                    self.fail(
                                        Some(&terminal),
                                        &format!("staging for {terminal}: {}", error.detail),
                                    );
                                    return;
                                }
                            }
                        } else {
                            self.retrying("recording the gate", |conn, now| {
                                peer_requests::record_gate_launch(
                                    conn,
                                    &self.id,
                                    &text,
                                    &terminal,
                                    seat.as_str(),
                                    now,
                                )
                            })
                        };
                        if opened {
                            (self.log)(&format!(
                                "peer launch {} from {}: launched seat {} on {terminal}",
                                short(&self.id),
                                self.tag,
                                seat.as_str()
                            ));
                        }
                        return;
                    }
                    (true, Some(pane)) => {
                        self.record("running", Some(&pane), None, None);
                        pane
                    }
                    // Killed before it answered: the target may exist.
                    _ if out.may_have_started() => {
                        return self.fail(
                            None,
                            &format!(
                                "creation: {}; a tab or terminal may have been created in {} here",
                                out.error,
                                if workspace.is_some() {
                                    format!("workspace {target}")
                                } else {
                                    "the herdr-core".to_owned()
                                }
                            ),
                        );
                    }
                    _ => {
                        return self.fail(None, &format!("creation: {}", out.error));
                    }
                }
            }
            _ => target,
        };
        let out = self.child(&launch_child_args(&pane, &self.id, plan));
        let seat = crate::cli::launch::json_envelope_seat(&out.stdout);
        match (out.ok, seat) {
            (true, Some(seat)) => {
                self.record("launched", Some(&pane), Some(&seat), None);
                (self.log)(&format!(
                    "peer launch {} from {}: launched seat {seat} on {pane}",
                    short(&self.id),
                    self.tag
                ));
            }
            _ => {
                let (detail, _) = launch_failure(&out, &pane);
                self.fail(Some(&pane), &detail);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Kills: a launch child that inherits the daemon's caller pane, harness
    // session or Herdr pane variables (attribution to a stray session), or
    // loses what a login shell needs to find the harness.
    #[test]
    fn launch_children_keep_only_login_environment() {
        let vars = [
            ("HOME", "/home/u"),
            ("PATH", "/usr/bin"),
            ("LC_ALL", "C.UTF-8"),
            ("HERDR_PANE_ID", "w1:p1"),
            ("HERDR_ENV", "1"),
            ("HERDR_SOCKET_PATH", "/run/herdr.sock"),
            ("CLAUDE_CODE_SESSION_ID", "abc"),
            ("CODEX_SESSION_ID", "def"),
            ("CODEX_THREAD_ID", "ghi"),
            ("HERMES_SESSION_ID", "jkl"),
            ("ANTIGRAVITY_CONVERSATION_ID", "mno"),
            ("HERDR_THREADS_SEAT", "s1"),
        ]
        .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        let kept: Vec<String> = child_env(vars.into_iter())
            .into_iter()
            .map(|(k, _)| k.into_string().unwrap())
            .collect();
        assert_eq!(kept, ["HOME", "PATH", "LC_ALL"]);
    }

    // Kills: the launcher reading the launched seat from a path `launch --json`
    // never prints (every real peer launch recorded failed), and the two
    // child command lines not parsing as the commands they are meant to be.
    #[test]
    fn child_output_and_argv_match_the_real_commands() {
        let report =
            serde_json::json!({"outcome": "started", "seat": "snSGseat0001", "pane": "w1:p2"});
        let stdout = crate::cli::launch::json_envelope(report).to_string();
        assert_eq!(
            crate::cli::launch::json_envelope_seat(&stdout).as_deref(),
            Some("snSGseat0001")
        );
        assert_eq!(
            crate::cli::launch::json_envelope_seat(r#"{"seat":"x"}"#),
            None
        );
        let plan = crate::sync::frame::LaunchPlan {
            target: LaunchTarget::Pane {
                space: None,
                tab: None,
                pane: "w1:p2".into(),
            },
            kind: "claude".into(),
            argv: vec!["--model".into(), "x".into()],
            cwd: None,
            profile_dir: None,
            handoff_prompt: None,
            message: None,
        };
        let parse = |args: Vec<String>| {
            let mut argv = vec![
                "ht".to_owned(),
                "--json".into(),
                "--state-dir".into(),
                "/s".into(),
                "--host-endpoint".into(),
                "/h.sock".into(),
            ];
            argv.extend(args);
            crate::cli::commands::parse_argv_exact_seats(argv)
                .unwrap()
                .action
        };
        let crate::cli::commands::CliAction::Launch(launch) = parse(launch_args("w1:p2", &plan))
        else {
            panic!("a launch")
        };
        assert_eq!(launch.target.as_str(), "w1:p2");
        assert_eq!(launch.argv, ["--model", "x"]);
        let crate::cli::commands::CliAction::InternalPeerTargetCreate {
            spec,
            label,
            cwd,
            gate_argv,
            directory_context,
        } = parse(target_args(
            Some("w1"),
            " review ",
            "Q0V",
            "/work",
            &[],
            None,
        ))
        else {
            panic!("a tab creation")
        };
        assert_eq!(
            (spec, label.as_str(), cwd.to_str(), gate_argv.len()),
            (
                crate::cli::peer_launch::TargetSpec::Tab("w1".into()),
                "review [from Q0V]",
                Some("/work"),
                0
            )
        );
        assert_eq!(directory_context, None);
        let context = crate::protocol::handoff::LaunchDirectoryContext {
            profile_dir: "/p".into(),
            cwd: "/work".into(),
            env: [("PROFILE_ROOT".to_owned(), "/p".to_owned())].into(),
        };
        let with = target_args(Some("w1"), "x", "Q0V", "/work", &[], Some(&context));
        assert!(with.iter().any(|a| a.starts_with("--directory-context={")));
        let crate::cli::commands::CliAction::InternalPeerTargetCreate {
            directory_context, ..
        } = parse(with)
        else {
            panic!("a tab creation")
        };
        assert_eq!(directory_context, Some(context));
        let gate = vec!["/bin/ht".to_owned(), "--x".into()];
        let crate::cli::commands::CliAction::InternalPeerTargetCreate {
            spec, gate_argv, ..
        } = parse(target_args(None, "term", "Q0V", "/work", &gate, None))
        else {
            panic!("a terminal creation")
        };
        assert_eq!(spec, crate::cli::peer_launch::TargetSpec::Terminal);
        assert_eq!(gate_argv, gate);
        // The previous name parses to the same action.
        let mut old = target_args(Some("w1"), "x", "Q0V", "/work", &[], None);
        old[1] = "peer-tab-create".into();
        assert!(matches!(
            parse(old),
            crate::cli::commands::CliAction::InternalPeerTargetCreate { .. }
        ));
        assert_eq!(clamp_detail(&"é".repeat(600)).len(), 1024);
    }

    fn pane_plan(handoff: bool) -> crate::sync::frame::LaunchPlan {
        crate::sync::frame::LaunchPlan {
            target: LaunchTarget::Pane {
                space: None,
                tab: None,
                pane: "w1:p2".into(),
            },
            kind: "claude".into(),
            argv: vec!["--model".into(), "x".into()],
            cwd: None,
            profile_dir: None,
            handoff_prompt: None,
            message: handoff.then(|| {
                Box::new(crate::sync::handoff_block::SenderBlock {
                    v: 1,
                    ..Default::default()
                })
            }),
        }
    }

    // Kills: a handoff plan launched by the plain `launch` (the agent would start with no
    // staged message), and a child command line the real CLI does not parse back to the request
    // and pane it was built from.
    #[test]
    fn worker_runs_handoff_child_for_handoff_plan() {
        assert_eq!(
            handoff_launch_args("w1:p2", "req-1"),
            [
                "internal",
                "peer-handoff-launch",
                "--request=req-1",
                "--pane=w1:p2"
            ]
        );
        assert_eq!(
            launch_child_args("w1:p2", "req-1", &pane_plan(true)),
            handoff_launch_args("w1:p2", "req-1")
        );
        let mut argv = vec![
            "ht".to_owned(),
            "--json".into(),
            "--state-dir".into(),
            "/s".into(),
            "--host-endpoint".into(),
            "/h.sock".into(),
        ];
        argv.extend(handoff_launch_args("w1:p2", "req-1"));
        let crate::cli::commands::CliAction::InternalPeerHandoffLaunch { request, pane } =
            crate::cli::commands::parse_argv_exact_seats(argv)
                .unwrap()
                .action
        else {
            panic!("a handoff launch")
        };
        assert_eq!((request.as_str(), pane.as_str()), ("req-1", "w1:p2"));
    }

    // Kills: the plain launch losing its place for a plan without a block (bare `launch --pane
    // P@M` must stay a gate-only launch).
    #[test]
    fn bare_plan_still_runs_launch() {
        let plan = pane_plan(false);
        assert_eq!(
            launch_child_args("w1:p2", "req-1", &plan),
            launch_args("w1:p2", &plan)
        );
        assert_eq!(launch_args("w1:p2", &plan)[0], "launch");
    }

    // Kills: an adapter without handoff startup input accepted (the launched agent would start
    // with no task), and an adapter's own error replaced by the generic one.
    #[test]
    fn startup_input_verdict_passes_adapter_answers_through() {
        use crate::protocol::results::{ApiError, ErrorCode};
        assert_eq!(
            startup_input_verdict(Ok::<Option<()>, ApiError>(None)),
            Err("selected adapter has no handoff startup input".to_owned())
        );
        assert_eq!(
            startup_input_verdict(Err::<Option<()>, _>(ApiError::new(
                ErrorCode::Conflict,
                "startup prompt is occupied"
            ))),
            Err("startup prompt is occupied".to_owned())
        );
        assert_eq!(startup_input_verdict(Ok(Some(()))), Ok(()));
    }

    // Kills: an unregistered harness or a flag-like tab label taken on (a real
    // tab created before the launch child fails), or an oversized plan
    // answered as retryable instead of refused.
    #[test]
    fn plans_are_refused_before_anything_runs() {
        let plan = |kind: &str, label: &str| crate::sync::frame::LaunchPlan {
            target: LaunchTarget::NewTab {
                space: "w1".into(),
                label: label.into(),
            },
            kind: kind.into(),
            argv: Vec::new(),
            cwd: Some("/tmp".into()),
            profile_dir: None,
            handoff_prompt: None,
            message: None,
        };
        let registered = crate::harness::registry::builtins()
            .registrations()
            .iter()
            .map(|r| r.metadata().id)
            .find(|id| crate::harness::registry::builtins().agent(id).is_ok())
            .expect("a registered agent harness");
        assert_eq!(plan_refusal(&plan(registered, "review"), 10), None);
        assert!(
            plan_refusal(&plan("nonesuch", "review"), 10)
                .is_some_and(|why| why.contains("not registered here"))
        );
        assert!(
            plan_refusal(&plan(registered, " --label=x"), 10)
                .is_some_and(|why| why.contains("must not start with -"))
        );
        assert!(
            plan_refusal(
                &plan(registered, "review"),
                peer_requests::MAX_PLAN_BYTES + 1
            )
            .is_some_and(|why| why.contains("too large"))
        );
    }

    // Kills: a child that never started (an upgrade left `current_exe`
    // deleted) read as an unconfirmed start (the requester locked out for 7
    // days), and a child killed after it started read as a plain failure (a
    // rerun would launch a second agent).
    #[test]
    fn only_a_started_child_can_leave_an_unconfirmed_start() {
        let never = run_child(Command::new("/nonexistent/herdr-threads-test-child"));
        assert!(!never.spawned);
        assert!(!launch_failure(&never, "w1:p2").1);
        let killed = ChildOutput {
            ok: false,
            spawned: true,
            code: None,
            stdout: String::new(),
            error: "timed out".into(),
        };
        let (detail, unconfirmed) = launch_failure(&killed, "w1:p2");
        assert!(unconfirmed);
        assert!(detail.contains("inspect pane w1:p2"), "{detail}");
        let refused = ChildOutput {
            code: Some(2),
            error: "no such pane".into(),
            ..killed
        };
        assert_eq!(
            launch_failure(&refused, "w1:p2"),
            ("launch: no such pane".into(), false)
        );
    }

    // Kills: a cap that concurrent requests of one node all pass, a lost row
    // holding a slot, a duplicate id decided twice, and a slot never freed
    // when its worker ends.
    #[test]
    fn slots_are_capped_per_node_and_freed_with_their_guard() {
        let live: Live = Arc::default();
        let mut held: Vec<LiveGuard> = (0..MAX_RUNNING_PER_PEER)
            .map(|i| claim_slot(&live, &format!("id{i}"), "nodeA").unwrap())
            .collect();
        assert_eq!(
            claim_slot(&live, "id9", "nodeA").err(),
            Some(Claim::Full(MAX_RUNNING_PER_PEER))
        );
        assert!(
            claim_slot(&live, "idB", "nodeB").is_ok(),
            "another node's cap"
        );
        assert_eq!(
            claim_slot(&live, "id0", "nodeA").err(),
            Some(Claim::InFlight)
        );
        held.pop();
        assert!(
            claim_slot(&live, "id9", "nodeA").is_ok(),
            "a dropped guard frees its slot"
        );
        // Concurrent claims never exceed the cap.
        let race: Live = Arc::default();
        let won = std::thread::scope(|threads| {
            let runs: Vec<_> = (0..16)
                .map(|i| {
                    let race = &race;
                    threads.spawn(move || claim_slot(race, &format!("r{i}"), "nodeA").ok())
                })
                .collect();
            runs.into_iter()
                .filter_map(|r| r.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(won.len() as u64, MAX_RUNNING_PER_PEER);
    }

    // Kills: a failed worker-thread spawn dropping the request's slot before
    // its failed row is written (a repeat would read the row as lost, i.e.
    // unconfirmed, and lock the requester out), and a spawned thread not
    // getting the worker.
    #[test]
    fn a_worker_whose_thread_cannot_start_keeps_its_slot_until_recorded() {
        let live: Live = Arc::default();
        let slot = claim_slot(&live, "id1", "nodeA").unwrap();
        let refused = |_: ThreadBody| Err(std::io::Error::other("no threads"));
        let Err((slot, error)) = hand_to_thread(slot, refused, |_slot: LiveGuard| {}) else {
            panic!("the spawn failed")
        };
        assert_eq!(error.to_string(), "no threads");
        assert!(live.lock().unwrap().contains_key("id1"), "still claimed");
        drop(slot);
        assert!(
            !live.lock().unwrap().contains_key("id1"),
            "freed once dropped"
        );
        let (seen, ran) = std::sync::mpsc::channel();
        hand_to_thread(7u32, spawn_named, move |value| seen.send(value).unwrap())
            .map_err(|(_, e)| e)
            .unwrap();
        assert_eq!(ran.recv().unwrap(), 7);
    }

    /// A scratch directory holding an executable `name`, for a PATH lookup.
    fn path_with(name: &str) -> (PathBuf, std::ffi::OsString) {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::test_support::scratch::temp_path(format!(
            "gate-argv-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(name);
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        (exe, dir.clone().into_os_string())
    }

    fn terminal_plan(kind: &str, argv: &[&str]) -> crate::sync::frame::LaunchPlan {
        crate::sync::frame::LaunchPlan {
            target: LaunchTarget::NewTerminal {
                label: "review".into(),
            },
            kind: kind.into(),
            argv: argv.iter().map(|a| (*a).to_owned()).collect(),
            cwd: Some("/tmp".into()),
            profile_dir: None,
            handoff_prompt: None,
            message: None,
        }
    }

    // Kills: a gate argv the real `internal launch-gate` cannot parse (the shim never ran
    // the harness), one without the state directory or the request's bootstrap, and a
    // harness executable that is not resolved from the plan's kind.
    #[test]
    fn gate_argv_parses_as_the_real_launch_gate() {
        let (exe, path) = path_with("claude");
        let plan = terminal_plan("claude", &["--model", "x"]);
        let argv = gate_argv(
            Path::new("/bin/ht"),
            Path::new("/state dir"),
            "req-1",
            &plan,
            Some(&path),
            None,
            None,
        )
        .unwrap();
        let crate::cli::commands::CliAction::InternalLaunchGate {
            bootstrap,
            config_roots,
            argv: harness,
        } = crate::cli::commands::parse_argv_exact_seats(argv.clone())
            .unwrap()
            .action
        else {
            panic!("a launch gate: {argv:?}")
        };
        assert_eq!(bootstrap, "req-1:1");
        assert!(config_roots.is_empty());
        assert_eq!(harness[0], exe.to_str().unwrap());
        assert_eq!(&harness[harness.len() - 2..], ["--model", "x"]);
        let state = argv.iter().position(|a| a == "--state-dir").unwrap();
        let internal = argv.iter().position(|a| a == "internal").unwrap();
        assert!(
            state + 1 < internal && argv[state + 1] == "/state dir",
            "{argv:?}"
        );
    }

    // Kills: a caller's config root dropped from the gate argv, or placed after `--` where the
    // harness would receive it instead of the gate.
    #[test]
    fn gate_argv_carries_the_callers_config_root() {
        let (_, path) = path_with("claude");
        let plan = terminal_plan("claude", &[]);
        let argv = gate_argv(
            Path::new("/bin/ht"),
            Path::new("/s"),
            "req-1",
            &plan,
            Some(&path),
            Some(Path::new("/profiles/b")),
            None,
        )
        .unwrap();
        let crate::cli::commands::CliAction::InternalLaunchGate { config_roots, .. } =
            crate::cli::commands::parse_argv_exact_seats(argv.clone())
                .unwrap()
                .action
        else {
            panic!("a launch gate: {argv:?}")
        };
        assert_eq!(config_roots, ["claude=/profiles/b"]);
    }

    // Kills: an unresolvable harness executable launching a gate that execs nothing.
    #[test]
    fn gate_argv_refuses_an_unresolvable_executable() {
        let plan = terminal_plan("claude", &[]);
        let empty = std::ffi::OsString::new();
        let error = gate_argv(
            Path::new("/bin/ht"),
            Path::new("/s"),
            "r",
            &plan,
            Some(&empty),
            None,
            None,
        )
        .unwrap_err();
        assert!(error.contains("no executable `claude`"), "{error}");
    }

    // Kills: the host's own launch arguments (Codex's `--no-daemon`) dropped from a
    // herdr-core launch, or placed after the plan's arguments.
    #[test]
    fn gate_argv_carries_host_launch_argv() {
        let (_, path) = path_with("codex");
        let plan = terminal_plan("codex", &["--model", "x"]);
        let argv = gate_argv(
            Path::new("/bin/ht"),
            Path::new("/s"),
            "r",
            &plan,
            Some(&path),
            None,
            None,
        )
        .unwrap();
        let at = |needle: &str| argv.iter().position(|a| a == needle).unwrap();
        assert!(at("--no-daemon") < at("--model"), "{argv:?}");
    }

    // Kills: a handoff gate that drops the frozen prompt or the profile directory, passes the
    // prompt outside the adapter's template slot, or lets either leak into a bare plan.
    #[test]
    fn gate_argv_carries_frozen_prompt_and_config_root() {
        let (exe, path) = path_with("claude");
        let plan = terminal_plan("claude", &["--model", "x"]);
        let prompt = "do the thing \u{2014} from: me (s1)";
        let argv = gate_argv(
            Path::new("/bin/ht"),
            Path::new("/s"),
            "req-1",
            &plan,
            Some(&path),
            Some(Path::new("/profiles/b")),
            Some(prompt),
        )
        .unwrap();
        let crate::cli::commands::CliAction::InternalLaunchGate {
            config_roots,
            argv: harness,
            ..
        } = crate::cli::commands::parse_argv_exact_seats(argv.clone())
            .unwrap()
            .action
        else {
            panic!("a launch gate: {argv:?}")
        };
        assert_eq!(config_roots, ["claude=/profiles/b"]);
        let registry = crate::harness::registry::builtins();
        let registration = registry.by_id(registry.agent("claude").unwrap()).unwrap();
        let template = registration
            .launch_policy()
            .unwrap()
            .prepare_startup_input(
                &plan.argv,
                &crate::harness::adapter::StartupInputSpec {
                    max_text_bytes: crate::ports::NativeLaunchRequest::MAX_ARG_BYTES,
                },
            )
            .unwrap()
            .unwrap();
        let mut expected = vec![exe.to_str().unwrap().to_owned()];
        expected.extend(registration.host_launch_argv(crate::ports::HostKind::Hc));
        expected.extend(template.apply(&plan.argv, prompt).unwrap().0);
        assert_eq!(harness, expected);
        assert!(harness.iter().any(|a| a.contains(prompt)), "{harness:?}");
    }

    // Kills: a bare launch whose gate argv changed when the handoff parameters were added.
    #[test]
    fn gate_argv_bare_plan_unchanged() {
        let (exe, path) = path_with("claude");
        let plan = terminal_plan("claude", &["--model", "x"]);
        let argv = gate_argv(
            Path::new("/bin/ht"),
            Path::new("/s"),
            "r",
            &plan,
            Some(&path),
            None,
            None,
        )
        .unwrap();
        let registry = crate::harness::registry::builtins();
        let registration = registry.by_id(registry.agent("claude").unwrap()).unwrap();
        let mut tail = vec![exe.to_str().unwrap().to_owned()];
        tail.extend(registration.host_launch_argv(crate::ports::HostKind::Hc));
        tail.extend(plan.argv.clone());
        assert_eq!(
            argv,
            [
                "/bin/ht",
                "--state-dir",
                "/s",
                "internal",
                "launch-gate",
                "--bootstrap",
                "r:1",
                "--"
            ]
            .iter()
            .map(|a| (*a).to_owned())
            .chain(tail)
            .collect::<Vec<_>>()
        );
    }
}
