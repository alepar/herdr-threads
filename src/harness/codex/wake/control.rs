#[cfg(target_os = "linux")]
use super::capture::from_fd;
use super::capture::{Descriptor, identity};
use crate::{
    harness::wake::{ControlFailure, OwnedWakeEntries, WakeAddOutcome, WakeControlContext},
    protocol::results::{ApiError, ErrorCode},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::{fs::DirBuilderExt, process::CommandExt},
    },
    path::PathBuf,
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio},
    time::{Duration, Instant},
};

pub(super) fn error(detail: &str) -> ApiError {
    ApiError::new(ErrorCode::TargetUnsafe, detail)
}
pub(super) fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && value.bytes().all(|b| b.is_ascii_graphic())
}

struct Home(PathBuf);
impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Peer<'a> {
    child: super::reap::Slot<'static>,
    input: ChildStdin,
    output: ChildStdout,
    stderr: ChildStderr,
    buffer: Vec<u8>,
    total: usize,
    errors: usize,
    seq: u64,
    effect_written: bool,
    context: &'a WakeControlContext<'a>,
    deadline: Instant,
    _home: Home,
    cleanup_done: bool,
    stdout_eof: bool,
    stderr_eof: bool,
}
impl Drop for Peer<'_> {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
fn nonblocking(pipe: &impl AsRawFd) -> Result<(), ApiError> {
    let fd = pipe.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        Err(error("queue helper pipe unavailable"))
    } else {
        Ok(())
    }
}
impl<'a> Peer<'a> {
    fn child_id(&self) -> u32 {
        self.child.as_ref().expect("owned queue helper").id()
    }
    fn start(
        d: &Descriptor,
        context: &'a WakeControlContext<'a>,
    ) -> Result<Self, Box<ControlFailure>> {
        let left = context
            .budget
            .deadline
            .0
            .saturating_sub(context.clock.monotonic_now().0);
        if left < 500 || context.budget.is_exhausted(context.clock) {
            return Err(error("insufficient queue helper budget").into());
        }
        let sqlite = d
            .database
            .path
            .parent()
            .and_then(|path| path.to_str())
            .ok_or_else(|| error("invalid queue storage encoding"))?;
        let config = format!("sqlite_home={}", toml_edit::Value::from(sqlite));
        let started = Instant::now();
        let deadline = started + Duration::from_millis(left.min(2000).saturating_sub(250));
        let parent = context.state_dir.join("queue-control");
        fs::create_dir_all(&parent).map_err(|_| error("queue helper scratch unavailable"))?;
        let path = parent.join(uuid::Uuid::new_v4().to_string());
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|_| error("queue helper scratch unavailable"))?;
        let home = Home(path);
        let mut command = Command::new(&d.executable.path);
        command
            .args(["-c", &config, "app-server", "--listen", "stdio://"])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &home.0)
            .env("CODEX_HOME", &home.0)
            .env("CODEX_SQLITE_HOME", sqlite)
            .env("CODEX_EXEC_SERVER_URL", "")
            .env("XDG_CONFIG_HOME", home.0.join("config"))
            .env("XDG_STATE_HOME", home.0.join("state"))
            .env("XDG_CACHE_HOME", home.0.join("cache"))
            .current_dir(&home.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        #[cfg(feature = "test-support")]
        crate::test_support::spawn::tag(&mut command);
        let mut child = super::reap::OWNED.reserve()?;
        *child = Some(
            command
                .spawn()
                .map_err(|_| error("queue helper could not start"))?,
        );
        let spawned = child.as_mut().expect("reserved child");
        let input = spawned.stdin.take().expect("piped input");
        let output = spawned.stdout.take().expect("piped output");
        let stderr = spawned.stderr.take().expect("piped stderr");
        let mut peer = Self {
            child,
            input,
            output,
            stderr,
            buffer: Vec::new(),
            total: 0,
            errors: 0,
            seq: 0,
            effect_written: false,
            context,
            deadline,
            _home: home,
            cleanup_done: false,
            stdout_eof: false,
            stderr_eof: false,
        };
        let readiness = (|| {
            if let Some(observer) = context.helper_observer {
                let witness = super::helper::witness(peer.child_id())?;
                observer.started(&witness)?;
            }
            nonblocking(&peer.input)?;
            nonblocking(&peer.output)?;
            nonblocking(&peer.stderr)?;
            let startup = deadline.min(started + Duration::from_millis(1000));
            peer.call_until("initialize",json!({"clientInfo":{"name":"herdr_threads_queue","version":"1"},"capabilities":{"experimentalApi":true}}),false,startup)?;
            peer.send(json!({"method":"initialized"}), false, startup)?;
            peer.verify_database(d)
        })();
        if let Err(e) = readiness {
            let cleanup = peer.finish();
            return Err(Box::new(finish_failure(e, cleanup)));
        }
        Ok(peer)
    }
    fn finish(&mut self) -> Option<ApiError> {
        if self.cleanup_done {
            return None;
        }
        let pipes_nonblocking =
            nonblocking(&self.output).is_ok() && nonblocking(&self.stderr).is_ok();
        self.finish_with(pipes_nonblocking, Child::try_wait)
    }
    fn finish_with(
        &mut self,
        pipes_nonblocking: bool,
        poll: impl FnMut(&mut Child) -> std::io::Result<Option<std::process::ExitStatus>>,
    ) -> Option<ApiError> {
        if self.cleanup_done {
            return None;
        }
        self.cleanup_done = true;
        let cleanup = self.finish_process(pipes_nonblocking, poll);
        let completion = self
            .context
            .helper_observer
            .and_then(|observer| observer.completed(cleanup.is_none()).err());
        cleanup.or(completion)
    }
    fn finish_process(
        &mut self,
        pipes_nonblocking: bool,
        mut poll: impl FnMut(&mut Child) -> std::io::Result<Option<std::process::ExitStatus>>,
    ) -> Option<ApiError> {
        let until = (Instant::now() + Duration::from_millis(250))
            .min(self.deadline + Duration::from_millis(250));
        // This group is exclusively ours. Never block in wait(): quiescence must be observed.
        let signal = unsafe { libc::kill(-(self.child_id() as i32), libc::SIGKILL) };
        let signal_failed =
            signal < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH);
        let child_pid = self.child_id();
        let failed = || {
            Some(error(&format!(
                "queue helper cleanup unconfirmed pid={}",
                child_pid
            )))
        };
        let mut reaped = false;
        let mut scratch = [0u8; 8192];
        if !pipes_nonblocking {
            return failed();
        }
        loop {
            if !reaped {
                match poll(self.child.as_mut().expect("owned queue helper")) {
                    Ok(Some(_)) => {
                        reaped = true;
                        *self.child = None;
                    }
                    Ok(None) => (),
                    Err(error) => {
                        if error.raw_os_error() == Some(libc::ECHILD) {
                            *self.child = None;
                        }
                        return failed();
                    }
                }
            }
            for (pipe, eof) in [
                (&mut self.output as &mut dyn Read, &mut self.stdout_eof),
                (&mut self.stderr as &mut dyn Read, &mut self.stderr_eof),
            ] {
                if *eof {
                    continue;
                }
                match pipe.read(&mut scratch) {
                    Ok(0) => *eof = true,
                    Ok(_) => (),
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                        ) => {}
                    Err(_) => return failed(),
                }
            }
            if reaped && self.stdout_eof && self.stderr_eof {
                return if signal_failed { failed() } else { None };
            }
            if Instant::now() >= until {
                return failed();
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn require_add_budget(&self) -> Result<(), ApiError> {
        self.check(self.deadline)?;
        // A refusal margin, not a promised round-trip latency.
        if self.deadline.saturating_duration_since(Instant::now()) < Duration::from_millis(100) {
            return Err(error("insufficient add request budget"));
        }
        Ok(())
    }
    fn preflight(&mut self, session: &str) -> Result<(), ApiError> {
        self.require_add_budget()?;
        let reply = self.call(
            "thread/queue/list",
            json!({"threadId":session,"limit":1}),
            false,
        )?;
        let rows = reply["data"]
            .as_array()
            .filter(|v| v.len() <= 1)
            .ok_or_else(|| error("invalid queue preflight"))?;
        for row in rows {
            if !row["id"].as_str().is_some_and(valid_id)
                || !row["clientUserMessageId"].is_string()
                || !row["input"].is_array()
            {
                return Err(error("invalid queue preflight"));
            }
        }
        match reply.get("nextCursor") {
            None | Some(Value::Null) => (),
            Some(Value::String(v)) if !v.is_empty() && v.len() <= 4096 => (),
            _ => return Err(error("invalid queue preflight cursor")),
        }
        self.require_add_budget()
    }
    fn check(&self, until: Instant) -> Result<(), ApiError> {
        if self.context.budget.is_exhausted(self.context.clock)
            || Instant::now() >= until.min(self.deadline)
        {
            Err(error("queue helper deadline or cancellation"))
        } else {
            Ok(())
        }
    }
    #[cfg(target_os = "macos")]
    fn helper_databases(&self) -> Result<std::collections::BTreeSet<(u64, u64)>, ApiError> {
        self.check(self.deadline)?;
        let until = (Instant::now() + Duration::from_millis(100)).min(self.deadline);
        let files = super::macos::open_vnodes(self.child_id(), until)
            .ok_or_else(|| error("queue helper files unavailable"))?;
        self.check(self.deadline)?;
        super::rules::helper_databases(&files, &identity).map_err(error)
    }
    #[cfg(target_os = "linux")]
    fn helper_databases(&self) -> Result<std::collections::BTreeSet<(u64, u64)>, ApiError> {
        let start = Instant::now();
        let mut found = std::collections::BTreeSet::new();
        for (i, entry) in fs::read_dir(format!("/proc/{}/fd", self.child_id()))
            .map_err(|_| error("queue helper files unavailable"))?
            .enumerate()
        {
            self.check(self.deadline)?;
            if i >= 1024 || start.elapsed() >= Duration::from_millis(100) {
                return Err(error("queue helper file observation limit"));
            }
            let entry = entry.map_err(|_| error("queue helper files unavailable"))?;
            let path =
                fs::read_link(entry.path()).map_err(|_| error("queue helper files changed"))?;
            if let Some(name) = path.file_name().and_then(|v| v.to_str())
                && name.starts_with("queue_")
                && (name.ends_with(".sqlite") || name.ends_with(".sqlite (deleted)"))
                && name != "queue_1.sqlite"
            {
                return Err(error("queue helper database ambiguous"));
            }
            if path.file_name().is_some_and(|v| v == "queue_1.sqlite") {
                let file =
                    from_fd(&entry.path()).ok_or_else(|| error("queue helper database changed"))?;
                found.insert((file.device, file.inode));
            }
        }
        Ok(found)
    }
    fn verify_database(&self, d: &Descriptor) -> Result<(), ApiError> {
        let found = self.helper_databases()?;
        if found.len() != 1
            || !found.contains(&(d.database.device, d.database.inode))
            || identity(&d.database.path).as_ref() != Some(&d.database)
        {
            return Err(error("queue helper storage mismatch"));
        }
        Ok(())
    }
    fn drain(&mut self) -> Result<bool, ApiError> {
        let mut bytes = [0; 8192];
        let mut eof = false;
        match self.output.read(&mut bytes) {
            Ok(0) => {
                eof = true;
                self.stdout_eof = true;
            }
            Ok(n) => {
                self.total += n;
                if self.total > 1048576 {
                    return Err(error("queue helper output limit"));
                }
                self.buffer.extend_from_slice(&bytes[..n]);
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) => {}
            Err(_) => return Err(error("queue helper output unavailable")),
        }
        // Enforce every complete and partial line before parsing JSON.
        if self
            .buffer
            .split(|b| *b == b'\n')
            .any(|line| line.len() > 262144)
        {
            return Err(error("queue helper line limit"));
        }
        match self.stderr.read(&mut bytes) {
            Ok(n) => {
                self.stderr_eof |= n == 0;
                self.errors += n;
                if self.errors > 8192 {
                    return Err(error("queue helper diagnostic limit"));
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) => {}
            Err(_) => return Err(error("queue helper diagnostics unavailable")),
        }
        Ok(eof)
    }
    fn send(&mut self, value: Value, effect: bool, until: Instant) -> Result<(), ApiError> {
        let mut bytes = serde_json::to_vec(&value).map_err(|_| error("invalid queue request"))?;
        bytes.push(b'\n');
        let mut offset = 0;
        while offset < bytes.len() {
            self.check(until)?;
            write_chunk(
                &mut self.input,
                &bytes,
                &mut offset,
                effect,
                &mut self.effect_written,
            )?;
            if self.drain()? {
                return Err(error("queue helper closed"));
            }
            if offset < bytes.len() {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        Ok(())
    }
    fn call_until(
        &mut self,
        method: &str,
        params: Value,
        effect: bool,
        until: Instant,
    ) -> Result<Value, ApiError> {
        self.seq += 1;
        let id = self.seq;
        self.send(
            json!({"id":id,"method":method,"params":params}),
            effect,
            until,
        )?;
        loop {
            self.check(until)?;
            while let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.buffer.drain(..=end).collect();
                let reply: Value =
                    serde_json::from_slice(&line).map_err(|_| error("malformed queue reply"))?;
                if reply.get("id").and_then(Value::as_u64) == Some(id) {
                    if reply.get("error").is_some() {
                        return Err(error("queue request rejected"));
                    }
                    return reply
                        .get("result")
                        .cloned()
                        .ok_or_else(|| error("missing queue result"));
                }
            }
            if self.drain()? && !self.buffer.contains(&b'\n') {
                return Err(error("queue helper closed"));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn call(&mut self, method: &str, params: Value, effect: bool) -> Result<Value, ApiError> {
        self.call_until(
            method,
            params,
            effect,
            self.deadline
                .min(Instant::now() + Duration::from_millis(750)),
        )
    }
}
fn write_chunk(
    writer: &mut impl Write,
    bytes: &[u8],
    offset: &mut usize,
    effect: bool,
    effect_written: &mut bool,
) -> Result<(), ApiError> {
    match writer.write(&bytes[*offset..]) {
        Ok(0) => Err(error("queue helper input closed")),
        Ok(n) => {
            *offset += n;
            *effect_written |= effect;
            Ok(())
        }
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
            ) =>
        {
            Ok(())
        }
        Err(_) => Err(error("queue helper input unavailable")),
    }
}
pub(super) fn enqueue(
    d: &Descriptor,
    session: &str,
    correlation: &str,
    verify: &mut dyn FnMut() -> Result<(), ApiError>,
    authorize: &mut dyn FnMut() -> Result<bool, ApiError>,
    context: &WakeControlContext<'_>,
) -> WakeAddOutcome {
    let no = |e: ApiError| WakeAddOutcome::RefusedBeforeEffect(e.into());
    if !valid_id(correlation) || !valid_id(session) {
        return no(error("invalid queue identity"));
    }
    if let Err(e) = verify() {
        return no(e);
    }
    let mut peer = match Peer::start(d, context) {
        Ok(v) => v,
        Err(e) => return WakeAddOutcome::RefusedBeforeEffect(*e),
    };
    if let Err(e) = peer.preflight(session) {
        let cleanup = peer.finish();
        return WakeAddOutcome::RefusedBeforeEffect(finish_failure(e, cleanup));
    }
    if let Err(e) = verify() {
        let cleanup = peer.finish();
        return WakeAddOutcome::RefusedBeforeEffect(finish_failure(e, cleanup));
    }
    if let Err(e) = peer.require_add_budget() {
        let cleanup = peer.finish();
        return WakeAddOutcome::RefusedBeforeEffect(finish_failure(e, cleanup));
    }
    match authorize() {
        Ok(true) => (),
        Ok(false) => {
            let cleanup = peer.finish();
            return WakeAddOutcome::RefusedBeforeEffect(finish_failure(
                error("queue wake no longer authorized"),
                cleanup,
            ));
        }
        Err(e) => {
            let cleanup = peer.finish();
            return WakeAddOutcome::RefusedBeforeEffect(finish_failure(e, cleanup));
        }
    }
    if let Err(e) = peer.require_add_budget() {
        let cleanup = peer.finish();
        return WakeAddOutcome::RefusedBeforeEffect(finish_failure(e, cleanup));
    }
    let result=peer.call("thread/queue/add",json!({"threadId":session,"clientUserMessageId":correlation,"input":[{"type":"text","text":crate::notification::policy::WAKE_UP}]}),true);
    let outcome = match result {
        Ok(reply) => match reply
            .get("queuedSubmission")
            .and_then(|v| v.get("id"))
            .and_then(Value::as_str)
            .filter(|v| valid_id(v))
        {
            Some(id) if reply["queuedSubmission"]["clientUserMessageId"] == correlation => {
                WakeAddOutcome::Accepted {
                    id: id.to_owned(),
                    cleanup_error: None,
                }
            }
            _ => WakeAddOutcome::Unknown(error("invalid accepted queue identity").into()),
        },
        Err(e) if peer.effect_written => WakeAddOutcome::Unknown(e.into()),
        Err(e) => no(e),
    };
    apply_cleanup(outcome, peer.finish())
}
pub(super) fn list_entries(
    d: &Descriptor,
    session: &str,
    correlation: &str,
    context: &WakeControlContext<'_>,
) -> Result<OwnedWakeEntries, Box<ControlFailure>> {
    if !valid_id(session) || !valid_id(correlation) {
        return Err(error("invalid queue identity").into());
    }
    let mut peer = Peer::start(d, context)?;
    let result = (|| {
        let mut cursor = None;
        let mut ids = Vec::new();
        let mut cursors = std::collections::BTreeSet::new();
        for _ in 0..4 {
            let reply = peer.call(
                "thread/queue/list",
                json!({"threadId":session,"limit":32,"cursor":cursor}),
                false,
            )?;
            let rows = reply["data"]
                .as_array()
                .filter(|v| v.len() <= 32)
                .ok_or_else(|| error("invalid queue list"))?;
            for row in rows {
                if row["clientUserMessageId"] == correlation {
                    let id = row["id"]
                        .as_str()
                        .filter(|v| valid_id(v))
                        .ok_or_else(|| error("invalid queue identity"))?;
                    ids.push(id.to_owned());
                }
            }
            match reply.get("nextCursor") {
                None | Some(Value::Null) => {
                    return Ok(OwnedWakeEntries {
                        ids,
                        complete: true,
                    });
                }
                Some(Value::String(next))
                    if !next.is_empty() && next.len() <= 4096 && cursors.insert(next.clone()) =>
                {
                    cursor = Some(next.clone())
                }
                _ => return Err(error("invalid queue cursor")),
            }
        }
        Ok(OwnedWakeEntries {
            ids,
            complete: false,
        })
    })();
    let cleanup = peer.finish();
    finish_read(result, cleanup)
}
pub(super) fn delete_entry(
    d: &Descriptor,
    session: &str,
    id: &str,
    context: &WakeControlContext<'_>,
) -> Result<bool, Box<ControlFailure>> {
    if !valid_id(session) || !valid_id(id) {
        return Err(error("invalid queue identity").into());
    }
    let mut peer = Peer::start(d, context)?;
    let result = peer
        .call(
            "thread/queue/delete",
            json!({"threadId":session,"queuedSubmissionId":id}),
            false,
        )
        .and_then(|reply| {
            reply["deleted"]
                .as_bool()
                .ok_or_else(|| error("invalid queue delete result"))
        });
    let cleanup = peer.finish();
    finish_read(result, cleanup)
}

fn finish_failure(error: ApiError, cleanup_error: Option<ApiError>) -> ControlFailure {
    ControlFailure {
        error,
        cleanup_error,
    }
}
fn finish_read<T>(
    result: Result<T, ApiError>,
    cleanup: Option<ApiError>,
) -> Result<T, Box<ControlFailure>> {
    match result {
        Ok(value) => match cleanup {
            None => Ok(value),
            Some(cleanup) => Err(Box::new(finish_failure(
                error("queue control cleanup unconfirmed"),
                Some(cleanup),
            ))),
        },
        Err(error) => Err(Box::new(finish_failure(error, cleanup))),
    }
}
fn apply_cleanup(outcome: WakeAddOutcome, cleanup: Option<ApiError>) -> WakeAddOutcome {
    match outcome {
        WakeAddOutcome::Accepted { id, cleanup_error } => WakeAddOutcome::Accepted {
            id,
            cleanup_error: cleanup_error.or(cleanup),
        },
        WakeAddOutcome::Unknown(mut e) => {
            e.cleanup_error = e.cleanup_error.or(cleanup);
            WakeAddOutcome::Unknown(e)
        }
        WakeAddOutcome::RefusedBeforeEffect(mut e) => {
            e.cleanup_error = e.cleanup_error.or(cleanup);
            WakeAddOutcome::RefusedBeforeEffect(e)
        }
    }
}

#[cfg(all(test, feature = "test-support"))]
#[path = "../../../../tests/harness/codex_wake_control.rs"]
mod tests;
