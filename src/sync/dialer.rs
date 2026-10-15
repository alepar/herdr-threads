//! The Dialer: dial tasks, the transport command, full-jitter backoff and
//! ownership of every sync child process (peer-link design, sections 13, 14
//! and 16).
//!
//! Responsibility (Dialer / DialTask / TransportCommand)
//! - Owns: one [`DialTask`]-style tokio task per enabled `sync_dial_targets`
//!   row, driven by a reload diff of `SyncStorePort::enabled_targets()`
//!   (new row: task; removed or disabled row: cancel; changed
//!   `remote_command`: restart). Each task owns its child process end to end:
//!   spawn with piped stdio, stderr in a 4 KiB ring, its own process group,
//!   `kill_on_drop`, and on every exit path (link ended, cancellation, daemon
//!   shutdown) `killpg(TERM)`, at most 2 s, `killpg(KILL)`, reap. It also owns
//!   the one place a `LinkError` or child exit becomes a `DialTransition`
//!   (`classify`), [`FullJitter`] and the enabled-target count idle exit reads.
//! - [`TransportCommand::build`] is the one place a transport argv is built:
//!   the ssh default, or `/bin/sh -c` over a [`DialTemplate`]
//!   (`sync.dial_command`) with every `{dest}` single-quoted for sh.
//! - Hides: the child, its pipes and process group, the stderr ring, backoff
//!   arithmetic and the reload rate limit (a kick-driven reload runs at most
//!   once a second; an explicit [`DialShared::request_reload`] runs at once).
//! - Must not: run the handshake or touch a link (the coordinator actor does,
//!   on the [`DialedStream`] a task hands it), read or write SQLite (only
//!   `SyncStorePort`), name a harness, or spawn anything else.
//! - Where new transports go: as data in [`TransportCommand::build`] or
//!   [`DialTemplate`], never as a second spawn site. A new way a dial can end
//!   is a branch in `classify`.
//! - Role: the dialing half of the sync coordinator. The actor constructs it
//!   ([`Dialer::start`]), nudges it on pacer kicks and shuts it down.
//!
//! A task trusts its own bookkeeping over the rows it is shown: a reload shows
//! the row as the store holds it, which can lag the task's own last write, so a
//! task waits on its own `next_attempt_at` and stop, and only a row that was
//! reset to `idle` after its last write (an operator re-add or a software
//! version change) ends a wait early.
//!
//! Core and harness-neutral.

use std::collections::BTreeMap;
use std::io;
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::{Notify, mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, sleep_until, timeout};

use super::coordinator::{DIAL_REPORT_SLOTS, DialReport, DialedStream, LogFn, uuid8};
use super::link::{LinkError, read_preamble};
use super::sync_store::SyncStorePort;
use crate::node::NodeUuid;
use crate::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant};
use crate::service::pacer::{scramble_seed, xorshift_unit};
use crate::store::sync_dial_targets::{
    DialState, DialTarget, DialTransition, LOCAL_VERSION, NOT_INSTALLED_RETRY_MS, StopReason,
    TargetId,
};

/// Longest `sync.dial_command`, in bytes.
pub const MAX_TEMPLATE_BYTES: usize = 4096;
/// The remote command when a target sets none. ssh hands it to the remote
/// login shell as one string.
pub const DEFAULT_REMOTE_COMMAND: &str =
    r#"exec "$(command -v herdr-threads || echo "$HOME/.local/bin/herdr-threads")" peer --stdio"#;
/// The most stderr a dial keeps (the tail), in bytes.
const STDERR_RING_BYTES: usize = 4096;
/// Longest stored `last_error`, in bytes.
const LAST_ERROR_BYTES: usize = 512;
/// How long a child may take to print the preamble (ssh connect plus login).
const PREAMBLE_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a child gets between `SIGTERM` and `SIGKILL`.
const TERM_GRACE: Duration = Duration::from_secs(2);
/// How long to wait for a child's exit status after the stream ended early.
const EXIT_STATUS_WAIT: Duration = Duration::from_secs(2);
/// How long to let the stderr reader drain after the child is gone.
const STDERR_DRAIN: Duration = Duration::from_millis(300);
/// A link that stayed up this long resets the failure count.
const UP_RESETS_ATTEMPTS: Duration = Duration::from_secs(60);
/// Kick-driven reloads run at most this often.
const RELOAD_SPACING: Duration = Duration::from_secs(1);
/// Cap of the full-jitter window, in milliseconds.
const JITTER_CAP_MS: u64 = 300_000;
/// Deadline of a store call, in milliseconds.
const STORE_CALL_MS: u64 = 5_000;
/// How long shutdown waits for every task's teardown.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// DialTemplate and TransportCommand
// ---------------------------------------------------------------------------

/// Why a `sync.dial_command` was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateError {
    TooLong,
    MissingDest,
    ForbiddenChar,
}

/// A validated `sync.dial_command`: at most [`MAX_TEMPLATE_BYTES`], at least
/// one `{dest}`, no NUL or newline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialTemplate(String);

impl DialTemplate {
    pub fn parse(text: &str) -> Result<Self, TemplateError> {
        if text.len() > MAX_TEMPLATE_BYTES {
            return Err(TemplateError::TooLong);
        }
        if text.contains(['\0', '\n']) {
            return Err(TemplateError::ForbiddenChar);
        }
        if !text.contains("{dest}") {
            return Err(TemplateError::MissingDest);
        }
        Ok(Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The template with every `{dest}` replaced, in one pass, by `dest`
    /// single-quoted for sh.
    fn render(&self, dest: &str) -> String {
        let quoted = sh_quote(dest);
        self.0.split("{dest}").collect::<Vec<_>>().join(&quoted)
    }
}

/// `text` as one sh word: single-quoted, each `'` written `'\''`.
fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The program and argv of one dial (see the module responsibility comment).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportCommand {
    pub program: String,
    pub argv: Vec<String>,
}

impl TransportCommand {
    /// The ssh default, or `/bin/sh -c <template>` when `template` is set (the
    /// target's `remote_command` is then ignored).
    pub fn build(target: &DialTarget, template: Option<&DialTemplate>) -> Self {
        match template {
            Some(template) => Self {
                program: "/bin/sh".to_owned(),
                argv: vec!["-c".to_owned(), template.render(&target.ssh_dest)],
            },
            None => {
                let remote = target
                    .remote_command
                    .as_deref()
                    .unwrap_or(DEFAULT_REMOTE_COMMAND);
                let mut argv: Vec<String> = [
                    "-o",
                    "BatchMode=yes",
                    "-o",
                    "ServerAliveInterval=15",
                    "-o",
                    "ServerAliveCountMax=3",
                    "-o",
                    "ControlMaster=no",
                    "-o",
                    "ControlPath=none",
                    "-o",
                    "ConnectTimeout=20",
                    "-T",
                    "--",
                ]
                .iter()
                .map(|part| (*part).to_owned())
                .collect();
                argv.push(target.ssh_dest.clone());
                argv.push(remote.to_owned());
                Self {
                    program: "ssh".to_owned(),
                    argv,
                }
            }
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command
            .args(&self.argv)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        command
    }
}

// ---------------------------------------------------------------------------
// FullJitter
// ---------------------------------------------------------------------------

/// `delay(n) = uniform(0, min(300 s, 1 s * 2^n))` on the xorshift step of
/// `service::pacer::Backoff`.
pub struct FullJitter {
    rng: u64,
}

impl FullJitter {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: scramble_seed(seed),
        }
    }

    pub fn delay(&mut self, attempts: u32) -> Duration {
        let window_ms = (1_000u64 << attempts.min(20)).min(JITTER_CAP_MS);
        let ms = (xorshift_unit(&mut self.rng) * window_ms as f64) as u64;
        Duration::from_millis(ms.min(window_ms.saturating_sub(1)))
    }
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// How a dial ended, as `classify` reads it.
struct DialEnd<'a> {
    why: &'a LinkError,
    /// The child printed the preamble.
    preamble_seen: bool,
    exit_code: Option<i32>,
    /// The last non-empty stderr line, if any.
    stderr_line: Option<&'a str>,
    up_for: Duration,
}

/// The only place a `LinkError` or child exit becomes a [`DialTransition`].
/// Returns it with the attempt count the task keeps.
fn classify(
    end: &DialEnd<'_>,
    attempts: u32,
    now_ms: i64,
    jitter: &mut FullJitter,
) -> (DialTransition, u32) {
    if let LinkError::PeerClosed(reason) | LinkError::Refused(reason) = end.why
        && let Ok(reason) = StopReason::try_from(*reason)
    {
        return (
            DialTransition::Stopped {
                reason,
                version: LOCAL_VERSION.to_owned(),
            },
            attempts,
        );
    }
    if !end.preamble_seen && end.exit_code == Some(127) {
        return (DialTransition::NotInstalled, attempts);
    }
    let base = if end.up_for >= UP_RESETS_ATTEMPTS {
        0
    } else {
        attempts
    };
    let delay = jitter.delay(base);
    let error = match end.stderr_line {
        Some(line) if !end.preamble_seen => line.to_owned(),
        _ => end.why.to_string(),
    };
    (
        DialTransition::Backoff {
            attempts: base + 1,
            next_at: now_ms.saturating_add(delay.as_millis() as i64),
            error: truncate_bytes(&error, LAST_ERROR_BYTES),
        },
        base + 1,
    )
}

fn truncate_bytes(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

// ---------------------------------------------------------------------------
// The child
// ---------------------------------------------------------------------------

type Ring = Arc<Mutex<Vec<u8>>>;

fn read_stderr(mut stderr: ChildStderr, ring: Ring) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf = [0u8; 1024];
        loop {
            match stderr.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut ring = ring.lock().unwrap_or_else(PoisonError::into_inner);
                    ring.extend_from_slice(&buf[..n]);
                    let over = ring.len().saturating_sub(STDERR_RING_BYTES);
                    if over > 0 {
                        ring.drain(..over);
                    }
                }
            }
        }
    })
}

fn signal_group(pgid: i32, signal: libc::c_int) {
    // SAFETY: `killpg` takes plain integers and has no memory effects.
    unsafe {
        libc::killpg(pgid, signal);
    }
}

/// A spawned dial child that leads its own process group. [`Self::teardown`]
/// is awaited on every exit path; `Drop` and `kill_on_drop` are the backstop.
struct ChildGuard {
    child: Child,
    pgid: i32,
    ring: Ring,
    reader: Option<JoinHandle<()>>,
    status: Option<ExitStatus>,
    done: bool,
}

impl ChildGuard {
    fn spawn(command: &TransportCommand) -> io::Result<(Self, ChildStdout, ChildStdin)> {
        let mut child = command.command().spawn()?;
        let pgid = child.id().map_or(0, |pid| pid as i32);
        let missing = || io::Error::other("child stdio not piped");
        let stdout = child.stdout.take().ok_or_else(missing)?;
        let stdin = child.stdin.take().ok_or_else(missing)?;
        let stderr = child.stderr.take().ok_or_else(missing)?;
        let ring: Ring = Arc::default();
        let reader = Some(read_stderr(stderr, ring.clone()));
        Ok((
            Self {
                child,
                pgid,
                ring,
                reader,
                status: None,
                done: false,
            },
            stdout,
            stdin,
        ))
    }

    /// `SIGTERM` to the group, up to [`TERM_GRACE`] for the leader to exit,
    /// `SIGKILL` to the group, reap. Idempotent.
    async fn teardown(&mut self) {
        if self.done {
            return;
        }
        self.done = true;
        if self.status.is_none()
            && let Ok(Some(status)) = self.child.try_wait()
        {
            self.status = Some(status);
        }
        if self.status.is_none() && self.pgid > 0 {
            signal_group(self.pgid, libc::SIGTERM);
            match timeout(TERM_GRACE, self.child.wait()).await {
                Ok(waited) => self.status = waited.ok(),
                Err(_) => {
                    signal_group(self.pgid, libc::SIGKILL);
                    self.status = self.child.wait().await.ok();
                }
            }
            // The leader is reaped, so the group id is held only by members
            // still running: sweep them.
            signal_group(self.pgid, libc::SIGKILL);
        }
        if let Some(mut reader) = self.reader.take()
            && timeout(STDERR_DRAIN, &mut reader).await.is_err()
        {
            reader.abort();
        }
    }

    fn exit_code(&self) -> Option<i32> {
        self.status.and_then(|status| status.code())
    }

    /// The last non-empty stderr line, at most [`LAST_ERROR_BYTES`].
    fn last_stderr_line(&self) -> Option<String> {
        let ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
        let text = String::from_utf8_lossy(&ring);
        let line = text.lines().rev().map(str::trim).find(|l| !l.is_empty())?;
        Some(truncate_bytes(line, LAST_ERROR_BYTES))
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !self.done && self.pgid > 0 {
            signal_group(self.pgid, libc::SIGKILL);
        }
        if let Some(reader) = self.reader.take() {
            reader.abort();
        }
    }
}

// ---------------------------------------------------------------------------
// DialTask
// ---------------------------------------------------------------------------

/// What every task shares.
struct TaskCtx {
    store: Arc<dyn SyncStorePort>,
    clock: Arc<dyn Clock>,
    log: LogFn,
    dialed: mpsc::Sender<DialedStream>,
    template: Option<DialTemplate>,
}

/// A task's own view of its target.
struct TaskState {
    /// Dial again no earlier than this (unix ms).
    next_at: Option<i64>,
    /// Stopped by a stop-class close: idle until the row is reset.
    stopped: bool,
    attempts: u32,
    learned: Option<NodeUuid>,
    /// The clock reading of this task's last store write.
    last_write_ms: i64,
    noise_logged: bool,
}

impl TaskState {
    fn from_row(row: &DialTarget) -> Self {
        Self {
            next_at: row.next_attempt_at,
            stopped: row.state == DialState::Stopped,
            attempts: row.attempts,
            learned: row.learned_node,
            last_write_ms: 0,
            noise_logged: false,
        }
    }

    /// An operator re-add (or a version change) resets the row to `idle` after
    /// this task's last write; that ends any wait.
    fn reset_by(&mut self, row: &DialTarget) -> bool {
        if row.state == DialState::Idle && row.updated_at >= self.last_write_ms {
            self.stopped = false;
            self.next_at = None;
            self.attempts = 0;
            return true;
        }
        false
    }
}

async fn record(ctx: &TaskCtx, target: TargetId, dest: &str, transition: DialTransition) -> i64 {
    let now = ctx.clock.utc_now();
    let store = ctx.store.clone();
    let clock = ctx.clock.clone();
    let result = tokio::task::spawn_blocking(move || {
        let budget = CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0.saturating_add(STORE_CALL_MS)),
            cancellation: Cancellation::default(),
        };
        store.record_transition(target, &transition, now, &budget)
    })
    .await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => (ctx.log)(&format!(
            "sync peer {dest}: could not record dial state: {:?}",
            e.code
        )),
        Err(_) => (ctx.log)(&format!(
            "sync peer {dest}: dial state write did not finish"
        )),
    }
    now.0
}

/// Waits out `next_at` or a stop. `false` when cancelled (or the supervisor
/// dropped the task's row feed).
async fn wait_phase(
    ctx: &TaskCtx,
    state: &mut TaskState,
    rows: &mut watch::Receiver<DialTarget>,
    cancel: &Cancellation,
) -> bool {
    rows.borrow_and_update();
    loop {
        let pause = if state.stopped {
            None
        } else {
            match state.next_at {
                None => return true,
                Some(at) => {
                    let wait = at - ctx.clock.utc_now().0;
                    if wait <= 0 {
                        return true;
                    }
                    Some(Duration::from_millis(wait as u64))
                }
            }
        };
        let due = async {
            match pause {
                Some(wait) => sleep(wait).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            () = cancel.cancelled() => return false,
            () = due => return true,
            changed = rows.changed() => {
                if changed.is_err() {
                    return false;
                }
                let row = rows.borrow_and_update().clone();
                if state.reset_by(&row) {
                    return true;
                }
            }
        }
    }
}

/// How one dial ended, before classification.
struct Ended {
    why: LinkError,
    preamble_seen: bool,
    up_for: Duration,
}

/// Preamble, hand-off and the report loop of one dial.
async fn session(
    ctx: &TaskCtx,
    state: &mut TaskState,
    target: &DialTarget,
    guard: &mut ChildGuard,
    mut stdout: ChildStdout,
    stdin: ChildStdin,
) -> Ended {
    let dest = &target.ssh_dest;
    match read_preamble(&mut stdout, Instant::now() + PREAMBLE_TIMEOUT).await {
        Ok(noise) => {
            if noise > 0 && !state.noise_logged {
                state.noise_logged = true;
                (ctx.log)(&format!(
                    "sync peer {dest}: remote printed {noise} bytes before preamble"
                ));
            }
        }
        Err(why) => {
            if matches!(why, LinkError::PreambleEof)
                && let Ok(Ok(status)) = timeout(EXIT_STATUS_WAIT, guard.child.wait()).await
            {
                guard.status = Some(status);
            }
            return Ended {
                why,
                preamble_seen: false,
                up_for: Duration::ZERO,
            };
        }
    }
    let (report_tx, mut reports) = mpsc::channel(DIAL_REPORT_SLOTS);
    let stream = DialedStream {
        target_id: target.target_id,
        dest: dest.clone(),
        stdout: Box::new(stdout),
        stdin: Box::new(stdin),
        report: report_tx,
    };
    if ctx.dialed.send(stream).await.is_err() {
        return Ended {
            why: LinkError::Shutdown,
            preamble_seen: true,
            up_for: Duration::ZERO,
        };
    }
    let mut established_at: Option<Instant> = None;
    loop {
        match reports.recv().await {
            Some(DialReport::Established { node_uuid }) => {
                established_at = Some(Instant::now());
                if let Some(old) = state.learned
                    && old != node_uuid
                {
                    (ctx.log)(&format!(
                        "sync peer {dest}: node changed {} -> {}",
                        uuid8(&old),
                        uuid8(&node_uuid)
                    ));
                }
                state.learned = Some(node_uuid);
                state.last_write_ms = record(
                    ctx,
                    target.target_id,
                    dest,
                    DialTransition::Linked { node: node_uuid },
                )
                .await;
            }
            Some(DialReport::LinkEnded { why, up_for, .. }) => {
                return Ended {
                    why,
                    preamble_seen: true,
                    up_for,
                };
            }
            None => {
                return Ended {
                    why: LinkError::Shutdown,
                    preamble_seen: true,
                    up_for: established_at.map_or(Duration::ZERO, |at| at.elapsed()),
                };
            }
        }
    }
}

/// One dial attempt from `Dialing` to its classified transition. `false` when
/// the task was cancelled (the child is already torn down).
async fn attempt(
    ctx: &TaskCtx,
    state: &mut TaskState,
    target: &DialTarget,
    jitter: &mut FullJitter,
    cancel: &Cancellation,
) -> bool {
    let dest = &target.ssh_dest;
    state.last_write_ms = record(ctx, target.target_id, dest, DialTransition::Dialing).await;
    let command = TransportCommand::build(target, ctx.template.as_ref());
    let (mut guard, stdout, stdin) = match ChildGuard::spawn(&command) {
        Ok(spawned) => spawned,
        Err(e) => {
            let (transition, attempts) = classify(
                &DialEnd {
                    why: &LinkError::Io(e),
                    preamble_seen: false,
                    exit_code: None,
                    stderr_line: None,
                    up_for: Duration::ZERO,
                },
                state.attempts,
                ctx.clock.utc_now().0,
                jitter,
            );
            return finish(ctx, state, target, transition, attempts, cancel).await;
        }
    };
    let ended = tokio::select! {
        biased;
        () = cancel.cancelled() => None,
        ended = session(ctx, state, target, &mut guard, stdout, stdin) => Some(ended),
    };
    guard.teardown().await;
    let Some(ended) = ended else {
        return false;
    };
    if cancel.is_cancelled() {
        return false;
    }
    let stderr_line = guard.last_stderr_line();
    let (transition, attempts) = classify(
        &DialEnd {
            why: &ended.why,
            preamble_seen: ended.preamble_seen,
            exit_code: guard.exit_code(),
            stderr_line: stderr_line.as_deref(),
            up_for: ended.up_for,
        },
        state.attempts,
        ctx.clock.utc_now().0,
        jitter,
    );
    finish(ctx, state, target, transition, attempts, cancel).await
}

async fn finish(
    ctx: &TaskCtx,
    state: &mut TaskState,
    target: &DialTarget,
    transition: DialTransition,
    attempts: u32,
    cancel: &Cancellation,
) -> bool {
    if cancel.is_cancelled() {
        return false;
    }
    state.attempts = attempts;
    match &transition {
        DialTransition::Backoff { next_at, .. } => {
            state.next_at = Some(*next_at);
            state.stopped = false;
        }
        DialTransition::NotInstalled => {
            state.next_at = Some(ctx.clock.utc_now().0.saturating_add(NOT_INSTALLED_RETRY_MS));
            state.stopped = false;
        }
        DialTransition::Stopped { .. } => {
            state.next_at = None;
            state.stopped = true;
        }
        DialTransition::Dialing | DialTransition::Linked { .. } => {}
    }
    state.last_write_ms = record(ctx, target.target_id, &target.ssh_dest, transition).await;
    true
}

async fn run_task(
    ctx: Arc<TaskCtx>,
    target: DialTarget,
    mut rows: watch::Receiver<DialTarget>,
    cancel: Cancellation,
    seed: u64,
) {
    let mut state = TaskState::from_row(&target);
    let mut jitter = FullJitter::new(seed);
    loop {
        if !wait_phase(&ctx, &mut state, &mut rows, &cancel).await {
            return;
        }
        if !attempt(&ctx, &mut state, &target, &mut jitter, &cancel).await {
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// Dialer
// ---------------------------------------------------------------------------

/// What the coordinator composes the Dialer from.
#[derive(Clone, Debug, Default)]
pub struct DialConfig {
    /// `sync.dial_command`; `None` dials with ssh.
    pub template: Option<DialTemplate>,
    /// A fixed seed makes every task's jitter reproducible (tests).
    pub jitter_seed: Option<u64>,
}

/// The Dialer's collaborators.
pub struct DialerDeps {
    pub store: Arc<dyn SyncStorePort>,
    pub clock: Arc<dyn Clock>,
    pub log: LogFn,
    /// The coordinator actor's `DialedStream` input.
    pub dialed: mpsc::Sender<DialedStream>,
}

/// The Dialer's cloneable surface: the reload nudge and the enabled-target
/// count (what `SyncHandle` exposes).
pub struct DialShared {
    nudge: Notify,
    explicit: AtomicBool,
    count: AtomicUsize,
}

impl DialShared {
    /// A reload now, ignoring the once-a-second spacing of kick-driven ones.
    pub fn request_reload(&self) {
        self.explicit.store(true, Ordering::Release);
        self.nudge.notify_one();
    }

    /// A rate-limited reload (a pacer kick).
    fn kick(&self) {
        self.nudge.notify_one();
    }

    /// Enabled targets after the last reload diff (0 before the first).
    pub fn enabled_targets(&self) -> usize {
        self.count.load(Ordering::Acquire)
    }
}

/// See the module documentation.
pub struct Dialer {
    shared: Arc<DialShared>,
    cancel: Cancellation,
    join: Option<JoinHandle<()>>,
}

impl Dialer {
    /// Spawn the reload supervisor on the current runtime; its first reload
    /// runs at once.
    pub fn start(deps: DialerDeps, config: DialConfig) -> Self {
        let shared = Arc::new(DialShared {
            nudge: Notify::new(),
            explicit: AtomicBool::new(false),
            count: AtomicUsize::new(0),
        });
        let cancel = Cancellation::default();
        let supervisor = Supervisor {
            ctx: Arc::new(TaskCtx {
                store: deps.store,
                clock: deps.clock,
                log: deps.log,
                dialed: deps.dialed,
                template: config.template,
            }),
            seed: config.jitter_seed,
            shared: shared.clone(),
            cancel: cancel.clone(),
            tasks: BTreeMap::new(),
        };
        let join = tokio::spawn(supervisor.run());
        Self {
            shared,
            cancel,
            join: Some(join),
        }
    }

    pub fn shared(&self) -> Arc<DialShared> {
        self.shared.clone()
    }

    /// A pacer kick: reload, at most once a second.
    pub fn kick(&self) {
        self.shared.kick();
    }

    /// Cancel every task (each runs its kill-and-reap teardown) and wait for
    /// them, bounded.
    pub async fn shutdown(&mut self) {
        self.cancel.cancel();
        if let Some(join) = self.join.take() {
            let _ = timeout(SHUTDOWN_WAIT, join).await;
        }
    }
}

impl Drop for Dialer {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

struct TaskEntry {
    remote_command: Option<String>,
    rows: watch::Sender<DialTarget>,
    cancel: Cancellation,
    join: JoinHandle<()>,
}

impl TaskEntry {
    async fn stop(self) {
        self.cancel.cancel();
        let _ = self.join.await;
    }
}

struct Supervisor {
    ctx: Arc<TaskCtx>,
    seed: Option<u64>,
    shared: Arc<DialShared>,
    cancel: Cancellation,
    tasks: BTreeMap<TargetId, TaskEntry>,
}

impl Supervisor {
    async fn run(mut self) {
        self.reload().await;
        let mut last = Instant::now();
        loop {
            tokio::select! {
                biased;
                () = self.cancel.cancelled() => break,
                () = self.shared.nudge.notified() => {}
            }
            if !self.shared.explicit.swap(false, Ordering::AcqRel) {
                tokio::select! {
                    biased;
                    () = self.cancel.cancelled() => break,
                    () = sleep_until(last + RELOAD_SPACING) => {}
                }
            }
            self.reload().await;
            last = Instant::now();
        }
        self.stop_all().await;
    }

    async fn enabled(&self) -> Option<Vec<DialTarget>> {
        let store = self.ctx.store.clone();
        let clock = self.ctx.clock.clone();
        let read = tokio::task::spawn_blocking(move || {
            let budget = CallBudget {
                deadline: MonoInstant(clock.monotonic_now().0.saturating_add(STORE_CALL_MS)),
                cancellation: Cancellation::default(),
            };
            store.enabled_targets(&budget)
        });
        match timeout(Duration::from_millis(STORE_CALL_MS * 2), read).await {
            Ok(Ok(Ok(targets))) => Some(targets),
            Ok(Ok(Err(e))) => {
                (self.ctx.log)(&format!("sync: dial target reload failed: {:?}", e.code));
                None
            }
            _ => {
                (self.ctx.log)("sync: dial target reload did not finish");
                None
            }
        }
    }

    async fn reload(&mut self) {
        let Some(targets) = self.enabled().await else {
            return;
        };
        let gone: Vec<TargetId> = self
            .tasks
            .keys()
            .filter(|id| !targets.iter().any(|t| t.target_id == **id))
            .copied()
            .collect();
        for id in gone {
            if let Some(entry) = self.tasks.remove(&id) {
                entry.stop().await;
            }
        }
        for target in targets {
            let id = target.target_id;
            let restart = match self.tasks.get(&id) {
                Some(entry) if entry.remote_command == target.remote_command => {
                    entry.rows.send_if_modified(|current| {
                        if *current == target {
                            false
                        } else {
                            *current = target.clone();
                            true
                        }
                    });
                    false
                }
                Some(_) => true,
                None => false,
            };
            if restart && let Some(entry) = self.tasks.remove(&id) {
                entry.stop().await;
            }
            if !self.tasks.contains_key(&id) {
                let entry = self.spawn_task(target);
                self.tasks.insert(id, entry);
            }
        }
        self.shared.count.store(self.tasks.len(), Ordering::Release);
    }

    fn spawn_task(&self, target: DialTarget) -> TaskEntry {
        let (rows_tx, rows_rx) = watch::channel(target.clone());
        let cancel = Cancellation::default();
        let seed = match self.seed {
            Some(seed) => seed ^ (target.target_id.0 as u64).wrapping_mul(0xA24B_AED4_963E_E407),
            None => {
                let id = uuid::Uuid::new_v4().as_u128();
                (id as u64) ^ ((id >> 64) as u64)
            }
        };
        let remote_command = target.remote_command.clone();
        let join = tokio::spawn(run_task(
            self.ctx.clone(),
            target,
            rows_rx,
            cancel.clone(),
            seed,
        ));
        TaskEntry {
            remote_command,
            rows: rows_tx,
            cancel,
            join,
        }
    }

    async fn stop_all(&mut self) {
        let entries: Vec<TaskEntry> = std::mem::take(&mut self.tasks).into_values().collect();
        for entry in &entries {
            entry.cancel.cancel();
        }
        for entry in entries {
            let _ = entry.join.await;
        }
        self.shared.count.store(0, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::time::UtcMillis;
    use crate::sync::frame::CloseReason;
    use crate::sync::sync_store::MemorySyncStore;

    struct TestClock(Instant);
    impl Clock for TestClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(1_700_000_000_000 + self.0.elapsed().as_millis() as i64)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(self.0.elapsed().as_millis() as u64)
        }
    }

    fn target(dest: &str, remote_command: Option<&str>) -> DialTarget {
        let store = MemorySyncStore::new();
        store.add_target(dest);
        let mut row = store.targets().remove(0);
        row.remote_command = remote_command.map(str::to_owned);
        row
    }

    fn template(text: &str) -> DialTemplate {
        DialTemplate::parse(text).expect("valid template")
    }

    #[test]
    fn default_ssh_argv_exact() {
        let built = TransportCommand::build(&target("alice@host", None), None);
        assert_eq!(built.program, "ssh");
        let expected: Vec<&str> = vec![
            "-o",
            "BatchMode=yes",
            "-o",
            "ServerAliveInterval=15",
            "-o",
            "ServerAliveCountMax=3",
            "-o",
            "ControlMaster=no",
            "-o",
            "ControlPath=none",
            "-o",
            "ConnectTimeout=20",
            "-T",
            "--",
            "alice@host",
            r#"exec "$(command -v herdr-threads || echo "$HOME/.local/bin/herdr-threads")" peer --stdio"#,
        ];
        assert_eq!(built.argv, expected);
        let custom = TransportCommand::build(&target("h", Some("run-it")), None);
        assert_eq!(custom.argv.last().map(String::as_str), Some("run-it"));
        assert_eq!(custom.argv[custom.argv.len() - 2], "h");
    }

    #[test]
    fn dial_command_quotes_dest_with_quote_and_spaces() {
        let dest = "a b'c;$x";
        let built = TransportCommand::build(
            &target(dest, Some("ignored")),
            Some(&template("exec cat {dest} {dest}")),
        );
        assert_eq!(built.program, "/bin/sh");
        assert_eq!(
            built.argv,
            vec![
                "-c".to_owned(),
                r"exec cat 'a b'\''c;$x' 'a b'\''c;$x'".to_owned()
            ]
        );
        // The shell reads each quoted dest back as the one word it was.
        let out = std::process::Command::new("/bin/sh")
            .args(["-c", &template("printf '%s|' {dest} {dest}").render(dest)])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!("{dest}|{dest}|")
        );
        // A dest that spells the placeholder is not expanded again.
        assert_eq!(template("echo {dest}").render("{dest}"), "echo '{dest}'");
    }

    #[test]
    fn dial_template_parse_refusals() {
        assert_eq!(
            DialTemplate::parse("exec x {dest}").unwrap().as_str(),
            "exec x {dest}"
        );
        assert_eq!(
            DialTemplate::parse("exec x"),
            Err(TemplateError::MissingDest)
        );
        assert_eq!(
            DialTemplate::parse("a {dest}\nb"),
            Err(TemplateError::ForbiddenChar)
        );
        assert_eq!(
            DialTemplate::parse("a {dest}\0"),
            Err(TemplateError::ForbiddenChar)
        );
        let at_limit = format!("{{dest}}{}", "x".repeat(MAX_TEMPLATE_BYTES - 6));
        assert_eq!(at_limit.len(), MAX_TEMPLATE_BYTES);
        assert!(DialTemplate::parse(&at_limit).is_ok());
        assert_eq!(
            DialTemplate::parse(&format!("{at_limit}x")),
            Err(TemplateError::TooLong)
        );
    }

    #[test]
    fn full_jitter_bounds_fixed_seed() {
        let mut a = FullJitter::new(7);
        let mut b = FullJitter::new(7);
        let mut seen_nonzero = false;
        for n in 0..40u32 {
            let cap = Duration::from_millis((1_000u64 << n.min(20)).min(300_000));
            for _ in 0..50 {
                let delay = a.delay(n);
                assert!(
                    delay < cap,
                    "delay {delay:?} at n={n} must be under {cap:?}"
                );
                seen_nonzero |= delay > Duration::ZERO;
                assert_eq!(delay, b.delay(n), "the same seed repeats");
            }
        }
        assert!(seen_nonzero);
        let mut other = FullJitter::new(8);
        let mut first = FullJitter::new(7);
        let differs = (0..20).any(|_| other.delay(10) != first.delay(10));
        assert!(differs, "a different seed gives a different sequence");
        // Window grows: n=0 stays under 1 s, n=10 reaches past it.
        let mut grow = FullJitter::new(3);
        assert!((0..200).all(|_| grow.delay(0) < Duration::from_secs(1)));
        assert!((0..200).any(|_| grow.delay(10) > Duration::from_secs(1)));
    }

    fn io_error() -> LinkError {
        LinkError::Io(io::Error::other("boom"))
    }

    #[test]
    fn attempts_reset_after_60s_up() {
        let why = io_error();
        let end = |up_for| DialEnd {
            why: &why,
            preamble_seen: true,
            exit_code: None,
            stderr_line: None,
            up_for,
        };
        let mut jitter = FullJitter::new(1);
        let (short, attempts) = classify(&end(Duration::from_secs(59)), 5, 1_000, &mut jitter);
        assert_eq!(attempts, 6);
        assert!(matches!(short, DialTransition::Backoff { attempts: 6, .. }));
        let (long, attempts) = classify(&end(Duration::from_secs(60)), 5, 1_000, &mut jitter);
        assert_eq!(attempts, 1);
        match long {
            DialTransition::Backoff {
                attempts: 1,
                next_at,
                error,
            } => {
                assert!(
                    (1_000..2_000).contains(&next_at),
                    "n=0 window is under 1 s: {next_at}"
                );
                assert!(error.contains("boom"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn classify_stop_not_installed_and_stderr() {
        let mut jitter = FullJitter::new(1);
        for reason in [
            CloseReason::AuthFailed,
            CloseReason::SelfLink,
            CloseReason::NodeCollision,
            CloseReason::EnvelopeUnsupported,
        ] {
            for why in [LinkError::PeerClosed(reason), LinkError::Refused(reason)] {
                let (t, _) = classify(
                    &DialEnd {
                        why: &why,
                        preamble_seen: true,
                        exit_code: None,
                        stderr_line: None,
                        up_for: Duration::ZERO,
                    },
                    0,
                    0,
                    &mut jitter,
                );
                assert!(
                    matches!(t, DialTransition::Stopped { version, .. } if version == LOCAL_VERSION)
                );
            }
        }
        let busy = LinkError::PeerClosed(CloseReason::Busy);
        let (t, _) = classify(
            &DialEnd {
                why: &busy,
                preamble_seen: true,
                exit_code: None,
                stderr_line: None,
                up_for: Duration::ZERO,
            },
            0,
            0,
            &mut jitter,
        );
        assert!(
            matches!(t, DialTransition::Backoff { .. }),
            "retry-class close backs off"
        );
        // 127 before the preamble is not-installed; 127 after it is not.
        let eof = LinkError::PreambleEof;
        let (t, _) = classify(
            &DialEnd {
                why: &eof,
                preamble_seen: false,
                exit_code: Some(127),
                stderr_line: Some("sh: nope"),
                up_for: Duration::ZERO,
            },
            3,
            0,
            &mut jitter,
        );
        assert_eq!(t, DialTransition::NotInstalled);
        let (t, _) = classify(
            &DialEnd {
                why: &io_error(),
                preamble_seen: true,
                exit_code: Some(127),
                stderr_line: Some("x"),
                up_for: Duration::ZERO,
            },
            0,
            0,
            &mut jitter,
        );
        assert!(matches!(t, DialTransition::Backoff { .. }));
        // Before the preamble the stderr line is the error; after it, the link error.
        let long_line = "e".repeat(2_000);
        let (t, _) = classify(
            &DialEnd {
                why: &eof,
                preamble_seen: false,
                exit_code: Some(255),
                stderr_line: Some(&long_line),
                up_for: Duration::ZERO,
            },
            0,
            0,
            &mut jitter,
        );
        match t {
            DialTransition::Backoff { error, .. } => assert_eq!(error.len(), LAST_ERROR_BYTES),
            other => panic!("{other:?}"),
        }
    }

    // --- Dialer over a fake actor end ------------------------------------

    /// A silent child that prints the preamble and idles.
    const PREAMBLE_THEN_SLEEP: &str = r#": {dest}; printf 'HTPEER1\n'; exec sleep 600"#;

    struct Rig {
        store: Arc<MemorySyncStore>,
        dialer: Dialer,
        dialed: mpsc::Receiver<DialedStream>,
        clock: Arc<TestClock>,
    }

    fn rig(command: &str) -> Rig {
        let store = Arc::new(MemorySyncStore::new());
        let clock = Arc::new(TestClock(Instant::now()));
        let (tx, dialed) = mpsc::channel(4);
        let dialer = Dialer::start(
            DialerDeps {
                store: store.clone(),
                clock: clock.clone(),
                log: Arc::new(|_| {}),
                dialed: tx,
            },
            DialConfig {
                template: Some(template(command)),
                jitter_seed: Some(11),
            },
        );
        Rig {
            store,
            dialer,
            dialed,
            clock,
        }
    }

    async fn until(what: &str, mut cond: impl FnMut() -> bool) {
        for _ in 0..500 {
            if cond() {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
        panic!("not reached: {what}");
    }

    fn row(rig: &Rig, id: TargetId) -> DialTarget {
        rig.store
            .targets()
            .into_iter()
            .find(|t| t.target_id == id)
            .unwrap()
    }

    fn node(n: u128) -> NodeUuid {
        NodeUuid::from_u128(n)
    }

    async fn next_dial(rig: &mut Rig) -> DialedStream {
        timeout(Duration::from_secs(10), rig.dialed.recv())
            .await
            .expect("a dial arrives")
            .expect("the dialer is running")
    }

    #[tokio::test]
    async fn established_records_linked_before_link_ended() {
        let mut rig = rig(PREAMBLE_THEN_SLEEP);
        let id = rig.store.add_target("peer-a");
        rig.dialer.shared().request_reload();
        let dialed = next_dial(&mut rig).await;
        assert_eq!(dialed.target_id, id);
        assert_eq!(dialed.dest, "peer-a");
        dialed
            .report
            .send(DialReport::Established { node_uuid: node(9) })
            .await
            .unwrap();
        until("linked while the link is up", || {
            row(&rig, id).state == DialState::Linked
        })
        .await;
        assert_eq!(row(&rig, id).learned_node, Some(node(9)));
        // The link is still up: the row stays linked and nothing redials.
        sleep(Duration::from_millis(100)).await;
        assert_eq!(row(&rig, id).state, DialState::Linked);
        dialed
            .report
            .send(DialReport::LinkEnded {
                target_id: id,
                why: LinkError::PeerClosed(CloseReason::Busy),
                up_for: Duration::from_secs(1),
                peer_node: Some(node(9)),
            })
            .await
            .unwrap();
        until("backoff after the link ended", || {
            row(&rig, id).state == DialState::Backoff
        })
        .await;
        let after = row(&rig, id);
        assert_eq!(after.attempts, 1);
        assert!(after.next_attempt_at.unwrap() < rig.clock.utc_now().0 + 1_000 + 100);
        rig.dialer.shutdown().await;
    }

    #[tokio::test]
    async fn refused_handshake_records_no_linked() {
        let mut rig = rig(PREAMBLE_THEN_SLEEP);
        let id = rig.store.add_target("peer-b");
        rig.dialer.shared().request_reload();
        let dialed = next_dial(&mut rig).await;
        dialed
            .report
            .send(DialReport::LinkEnded {
                target_id: id,
                why: LinkError::Refused(CloseReason::AuthFailed),
                up_for: Duration::ZERO,
                peer_node: None,
            })
            .await
            .unwrap();
        until("stopped", || row(&rig, id).state == DialState::Stopped).await;
        let stopped = row(&rig, id);
        assert_eq!(stopped.learned_node, None);
        assert_eq!(stopped.last_linked_at, None);
        assert_eq!(
            stopped.stopped.as_ref().map(|s| s.0),
            Some(StopReason::AuthFailed)
        );
        rig.dialer.shutdown().await;
    }

    #[tokio::test]
    async fn stop_dialing_stopped_survives_reload_until_row_changes() {
        let mut rig = rig(PREAMBLE_THEN_SLEEP);
        let id = rig.store.add_target("peer-c");
        rig.dialer.shared().request_reload();
        let dialed = next_dial(&mut rig).await;
        dialed
            .report
            .send(DialReport::LinkEnded {
                target_id: id,
                why: LinkError::PeerClosed(CloseReason::EnvelopeUnsupported),
                up_for: Duration::ZERO,
                peer_node: None,
            })
            .await
            .unwrap();
        until("stopped", || row(&rig, id).state == DialState::Stopped).await;
        let version = row(&rig, id).stopped.clone().unwrap().1;
        assert_eq!(version.as_deref(), Some(LOCAL_VERSION));
        // Reloads change nothing while the row stays stopped.
        for _ in 0..3 {
            rig.dialer.shared().request_reload();
            sleep(Duration::from_millis(80)).await;
        }
        assert!(
            rig.dialed.try_recv().is_err(),
            "a stopped target is not dialed"
        );
        assert_eq!(row(&rig, id).state, DialState::Stopped);
        // The operator re-adds it: idle again, dialed again.
        rig.store.reset("peer-c");
        rig.dialer.shared().request_reload();
        let again = next_dial(&mut rig).await;
        assert_eq!(again.target_id, id);
        rig.dialer.shutdown().await;
    }

    #[tokio::test]
    async fn exit_127_not_installed_plus_30min() {
        let rig_ = rig(": {dest}; exit 127");
        let mut rig = rig_;
        let id = rig.store.add_target("peer-d");
        let before = rig.clock.utc_now().0;
        rig.dialer.shared().request_reload();
        until("not installed", || {
            row(&rig, id).state == DialState::NotInstalled
        })
        .await;
        let row = row(&rig, id);
        let at = row.next_attempt_at.expect("a retry time");
        let half_hour = 30 * 60 * 1000;
        assert!(
            at >= before + half_hour && at <= rig.clock.utc_now().0 + half_hour,
            "{at}"
        );
        assert!(rig.dialed.try_recv().is_err());
        rig.dialer.shutdown().await;
    }

    #[tokio::test]
    async fn reload_targets_picks_up_new_target() {
        let mut rig = rig(PREAMBLE_THEN_SLEEP);
        until("first reload", || {
            rig.dialer.shared().enabled_targets() == 0
        })
        .await;
        sleep(Duration::from_millis(50)).await;
        let id = rig.store.add_target("late");
        // No kick: only the explicit reload.
        rig.dialer.shared().request_reload();
        let dialed = next_dial(&mut rig).await;
        assert_eq!(dialed.target_id, id);
        rig.dialer.shutdown().await;
    }

    #[tokio::test]
    async fn enabled_target_count_follows_add_disable_forget() {
        let rig = rig("exec sleep 600; : {dest}");
        let shared = rig.dialer.shared();
        let mut rig = rig;
        assert_eq!(shared.enabled_targets(), 0);
        rig.store.add_target("one");
        shared.request_reload();
        until("one target", || shared.enabled_targets() == 1).await;
        rig.store.add_target("two");
        shared.request_reload();
        until("two targets", || shared.enabled_targets() == 2).await;
        rig.store.disable("one");
        shared.request_reload();
        until("disable lowers it", || shared.enabled_targets() == 1).await;
        // Forget disables the row and deletes nothing.
        rig.store.disable("two");
        shared.request_reload();
        until("forget lowers it", || shared.enabled_targets() == 0).await;
        assert_eq!(rig.store.targets().len(), 2);
        rig.dialer.shutdown().await;
        assert!(rig.dialed.try_recv().is_err());
    }
}
