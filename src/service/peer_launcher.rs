//! Launches an agent here for a linked peer node (remote launch, TRUST-POLICY
//! N15, A3 `peer_request:<node_uuid>`).
//!
//! Responsibility (PeerLaunchService)
//! - Owns: the decision to take a peer's `launch_request` (its target exists
//!   on one attached host and is not held, a new tab's workspace and `cwd`
//!   exist here), recording it in `peer_requests` before acting, and running
//!   it as children of this binary with a scrubbed environment: for a new tab
//!   `internal peer-tab-create` (label ending `[from <tag>]`), then the
//!   ordinary `launch --pane`, so the launch is exactly a local `launch`.
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
    vars.filter(|(key, _)| {
        let Some(key) = key.to_str() else {
            return false;
        };
        KEPT_ENV.contains(&key)
            || key.starts_with("LC_")
            || (cfg!(feature = "test-support")
                && (key.starts_with("HT_TEST_") || key == "HT_LEAK_RUN_ID"))
    })
    .collect()
}

pub struct PeerLaunchService {
    pub store: Arc<SqliteStore>,
    pub sessions: Arc<HostSessions>,
    pub writer: Arc<FairWriter>,
    pub clock: Arc<dyn Clock>,
    /// This node's `--state-dir`, passed to the children.
    pub state_dir: PathBuf,
    pub log: Arc<dyn Fn(&str) + Send + Sync>,
}

/// Where one accepted request launches.
struct Placement {
    session: Arc<HostSession>,
    /// The pane for `Pane`, the workspace for `NewTab`.
    target: String,
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
    fn place(&self, target: &LaunchTarget) -> Result<Placement, LaunchState> {
        let budget = self.budget();
        let mut found: Vec<Placement> = Vec::new();
        let mut failed = false;
        let sessions = self.sessions.attached();
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
                LaunchTarget::Unknown => {}
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
            let (_, inserted) = peer_requests::insert_or_get(conn, row)?;
            if inserted {
                peer_requests::record(conn, &row.id, "refused", None, None, Some(&detail), now)?;
            }
            Ok(())
        });
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
            return Ok(row.answer());
        }
        let plan = &request.plan;
        let new_tab = matches!(plan.target, LaunchTarget::NewTab { .. });
        let plan_text = serde_json::to_string(plan).unwrap_or_default();
        let row = PeerRequest {
            id: id.clone(),
            node_uuid: from.to_string(),
            requester_seat: (!request.requester_seat.is_empty())
                .then(|| clamp_detail(&request.requester_seat)),
            kind: if new_tab { "new_tab" } else { "pane" }.into(),
            plan: plan_text,
            state: "running".into(),
            pane: None,
            seat: None,
            detail: None,
            created_at: now_ms(self.clock.as_ref()),
            updated_at: 0,
        };
        if row.plan.len() > peer_requests::MAX_PLAN_BYTES {
            return Ok(self.refuse(from, &row, "launch plan too large"));
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
        let (stored, inserted) = self.write(|conn| peer_requests::insert_or_get(conn, &row))?;
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
            id,
            tag,
            store: Arc::clone(&self.store),
            writer: Arc::clone(&self.writer),
            clock: Arc::clone(&self.clock),
            state_dir: self.state_dir.clone(),
            endpoint: placement.session.locator().as_path().to_path_buf(),
            log: Arc::clone(&self.log),
        };
        let request = request.clone();
        let target = placement.target;
        std::thread::spawn(move || worker.run(&request, target));
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

/// The `internal peer-tab-create` arguments of a new tab: its label ends in
/// `[from <tag>]`, the requesting node.
pub fn tab_args(workspace: &str, label: &str, tag: &str, cwd: &str) -> Vec<String> {
    vec![
        "internal".into(),
        "peer-tab-create".into(),
        "--workspace".into(),
        workspace.into(),
        "--label".into(),
        format!("{} [from {tag}]", label.trim()),
        "--cwd".into(),
        cwd.into(),
    ]
}

/// The ordinary `launch --pane` arguments of `plan` on `pane`.
pub fn launch_args(pane: &str, plan: &crate::sync::frame::LaunchPlan) -> Vec<String> {
    let mut args = vec![
        "launch".to_owned(),
        "--pane".to_owned(),
        pane.to_owned(),
        "--kind".to_owned(),
        plan.kind.clone(),
    ];
    if !plan.argv.is_empty() {
        args.push("--".into());
        args.extend(plan.argv.iter().cloned());
    }
    args
}

/// Runs one accepted request to its outcome.
struct Worker {
    id: String,
    tag: String,
    store: Arc<SqliteStore>,
    writer: Arc<FairWriter>,
    clock: Arc<dyn Clock>,
    state_dir: PathBuf,
    endpoint: PathBuf,
    log: Arc<dyn Fn(&str) + Send + Sync>,
}

/// One finished child: exit success, stdout, the last stderr line.
struct ChildOutput {
    ok: bool,
    /// The exit status, when the child exited by itself.
    code: Option<i32>,
    stdout: String,
    error: String,
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
    fn record(&self, state: &str, pane: Option<&str>, seat: Option<&str>, detail: Option<&str>) {
        let budget = CallBudget {
            deadline: MonoInstant(self.clock.monotonic_now().0 + CALL_BUDGET_MS),
            cancellation: Cancellation::default(),
        };
        let now = now_ms(self.clock.as_ref());
        let written = self
            .writer
            .enter_background(&budget, self.clock.as_ref())
            .and_then(|_turn| {
                self.store.sync_operator_quantum(&budget, |tx, _, _| {
                    peer_requests::record(tx, &self.id, state, pane, seat, detail, now)
                        .map_err(crate::store::connection::store_error)
                })
            });
        if let Err(error) = written {
            (self.log)(&format!(
                "peer launch {}: recording {state} failed: {}",
                short(&self.id),
                error.detail
            ));
        }
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
            .arg(&self.endpoint)
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
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                return ChildOutput {
                    ok: false,
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
                code: status.code(),
                stdout,
                error: last,
            },
            None => ChildOutput {
                ok: false,
                code: None,
                stdout,
                error: "timed out".into(),
            },
        }
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

    fn run(&self, request: &LaunchRequest, target: String) {
        let plan = &request.plan;
        let pane = match &plan.target {
            LaunchTarget::NewTab { label, .. } => {
                let out = self.child(&tab_args(
                    &target,
                    label,
                    &self.tag,
                    plan.cwd.as_deref().unwrap_or_default(),
                ));
                let pane = serde_json::from_str::<serde_json::Value>(out.stdout.trim())
                    .ok()
                    .and_then(|v| v["pane"].as_str().map(str::to_owned));
                match (out.ok, pane) {
                    (true, Some(pane)) => {
                        self.record("running", Some(&pane), None, None);
                        pane
                    }
                    _ => {
                        return self.fail(None, &format!("tab creation: {}", out.error));
                    }
                }
            }
            _ => target,
        };
        let out = self.child(&launch_args(&pane, plan));
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
                let detail = if out.code == Some(crate::cli::launch::EXIT_START_UNCONFIRMED) {
                    format!("start unconfirmed: inspect pane {pane} here before launching again")
                } else if out.error.is_empty() {
                    "launch did not report a seat".to_owned()
                } else {
                    format!("launch: {}", out.error)
                };
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
        let crate::cli::commands::CliAction::InternalPeerTabCreate {
            workspace,
            label,
            cwd,
        } = parse(tab_args("w1", " review ", "Q0V", "/work"))
        else {
            panic!("a tab creation")
        };
        assert_eq!(
            (workspace.as_str(), label.as_str(), cwd.to_str()),
            ("w1", "review [from Q0V]", Some("/work"))
        );
        assert_eq!(clamp_detail(&"é".repeat(600)).len(), 1024);
    }
}
