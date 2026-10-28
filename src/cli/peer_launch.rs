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

/// The request id of this launch: derived from the frozen request and kept in
/// `node_dir/remote-launch/` until the launch settles, so a rerun of the same
/// command (after a link drop, a crash or the timeout) resumes it and never
/// launches twice; a new launch after it settled gets a new id.
fn request_id(
    node_dir: &Path,
    machine: &str,
    plan: &crate::sync::frame::LaunchPlan,
) -> std::io::Result<(uuid::Uuid, std::path::PathBuf)> {
    use sha2::Digest;
    let mut hash = sha2::Sha256::new();
    hash.update(b"herdr-threads remote launch v1\0");
    hash.update(machine.as_bytes());
    hash.update(b"\0");
    hash.update(serde_json::to_vec(plan).unwrap_or_default());
    let key = hash
        .finalize()
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let dir = node_dir.join("remote-launch");
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(format!("{key}.id"));
    if let Ok(text) = std::fs::read_to_string(&file)
        && let Ok(id) = text.trim().parse()
    {
        return Ok((id, file));
    }
    let id = uuid::Uuid::new_v4();
    let tmp = dir.join(format!("{key}.tmp"));
    std::fs::write(&tmp, id.to_string())?;
    std::fs::rename(&tmp, &file)?;
    Ok((id, file))
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

/// Runs a remote launch to its outcome: the launched seat (now held here as
/// that machine's) and its pane, or a refusal.
pub fn run_remote<C: crate::ports::LocalClient + ?Sized, W: Write>(
    remote: &RemoteLaunch,
    requester_seat: Option<&str>,
    node_dir: &Path,
    client: &C,
    clock: &dyn Clock,
    stderr: &mut W,
) -> Result<(String, String), RunError> {
    use crate::protocol::sync_admin::{LaunchReply, RemoteLaunchAnswer, SyncCommand, SyncReply};
    use crate::sync::frame::{LaunchRequest, LaunchState};
    let (id, file) = request_id(node_dir, &remote.machine, &remote.plan)?;
    let request = LaunchRequest {
        id,
        requester_seat: requester_seat.unwrap_or_default().to_owned(),
        plan: remote.plan.clone(),
    };
    let settle = || {
        let _ = std::fs::remove_file(&file);
    };
    let refuse =
        |code: ErrorCode, detail: String| -> RunError { ApiError::new(code, detail).into() };
    let started = std::time::Instant::now();
    let mut announced = false;
    loop {
        let budget = CallBudget {
            deadline: MonoInstant(
                clock
                    .monotonic_now()
                    .0
                    .saturating_add(super::seat_resolution::REMOTE_ADDRESS_BUDGET_MS),
            ),
            cancellation: Cancellation::default(),
        };
        let reply = client.call(
            crate::protocol::commands::Command::Sync(SyncCommand::RemoteLaunch {
                machine: remote.machine.clone(),
                request: Box::new(request.clone()),
            }),
            &budget,
        )?;
        let crate::protocol::results::CommandResult::Sync(SyncReply::Launch(reply)) = reply else {
            return Err(refuse(
                ErrorCode::InvalidRequest,
                "unexpected remote launch result".into(),
            ));
        };
        let (node, answer) = match reply {
            LaunchReply::Remote { node, answer } => (node, answer),
            LaunchReply::SelfNode => {
                settle();
                return Err(refuse(
                    ErrorCode::InvalidRequest,
                    format!(
                        "{} is this machine: launch without @MACHINE",
                        one_line(&remote.machine)
                    ),
                ));
            }
            LaunchReply::UnknownMachine { known } => {
                settle();
                let names: Vec<String> = known.iter().take(8).map(label_of).collect();
                return Err(refuse(
                    ErrorCode::NotFound,
                    format!(
                        "no machine {}; known: {}; for a local pane whose label has an @, pass its pane ID",
                        one_line(&remote.machine),
                        if names.is_empty() {
                            "none".to_owned()
                        } else {
                            names.join(", ")
                        }
                    ),
                ));
            }
            LaunchReply::AmbiguousMachine { matches } => {
                settle();
                let lines: Vec<String> = matches
                    .iter()
                    .map(|row| format!("  {} {}", row.uuid, label_of(row)))
                    .collect();
                return Err(refuse(
                    ErrorCode::Conflict,
                    format!(
                        "ambiguous machine {}: {} matches\n{}\npass @<uuid>",
                        one_line(&remote.machine),
                        matches.len(),
                        lines.join("\n")
                    ),
                ));
            }
            LaunchReply::Unknown => {
                return Err(refuse(
                    ErrorCode::InvalidRequest,
                    "the daemon answered in an unknown form".into(),
                ));
            }
        };
        let label = label_of(&node);
        let pending = match answer {
            RemoteLaunchAnswer::Answered { state } => match state {
                LaunchState::Launched { seat, pane } => {
                    settle();
                    writeln!(
                        stderr,
                        "launched: {} on {}@{} (request {})",
                        one_line(&seat),
                        one_line(&pane),
                        node.tag,
                        &id.to_string()[..8]
                    )?;
                    return Ok((seat, pane));
                }
                LaunchState::Refused { detail } => {
                    settle();
                    return Err(refuse(
                        ErrorCode::InvalidRequest,
                        format!("{label} refused the launch: {}", one_line(&detail)),
                    ));
                }
                LaunchState::Failed { detail, pane } => {
                    settle();
                    let left = pane.map_or_else(String::new, |p| {
                        format!("; it left tab pane {} there", one_line(&p))
                    });
                    return Err(refuse(
                        ErrorCode::HostUnavailable,
                        format!("launch on {label} failed: {}{left}", one_line(&detail)),
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
        if started.elapsed() >= REMOTE_LAUNCH_TIMEOUT {
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
