//! `LocateCaller`: daemon-side nearest-ancestor verification of a two-family caller.
//!
//! Responsibility (LocateCaller)
//! - Owns: the walk from the subject (the claim's liveness pid, else the
//!   transport peer pid) up `ProcessInfo` parents against each candidate's
//!   `HostPort::target_root_process`, the herdr-core terminal-root wall (a
//!   process whose argv[0] basename is in `host::hc::foreign_terminal_process_names`
//!   ends the walk: a root above it does not verify; unreadable argv is not a
//!   wall, an accepted F/N limit), and its cache keyed on (candidates, subject
//!   pid, subject start stamp, host set epoch) with unstamped subjects
//!   uncached; the server-environ read for the inherited-name refusal (M6):
//!   [`read_server_environ`] (a plain function with a Linux `/proc` body and a
//!   macOS `KERN_PROCARGS2` body, `None` on any failure) and
//!   [`inherited_name`], which asks the host that owns the hinted pane.
//! - Hides: how a pid maps to a host target.
//! - Must not: run for a `One` caller (that would be adversarial verification,
//!   which TRUST-POLICY forbids; the CLI sends this command only for
//!   `Ambiguous`), fall back to a candidate when none verifies, or fail closed
//!   on an unreadable server environ (`server_process() == None` skips that
//!   check).
//! - Relationships: delegates to `HostPort::target_root_process` of the
//!   sessions `HostSessions::all_for_reads` returns for a pane candidate (the hosts
//!   that do not answer terminal ids). A terminal candidate asks only the core its
//!   socket names (`CorePort`, attached on a miss); a socket that is not a core is
//!   `Unsupported`, one that cannot be attached `Unreachable`. With both families
//!   the nearest ancestor wins, whichever family owns it.
//! - Where new behaviour goes: a new host candidate verifies through its host's
//!   `target_root_process` answer, with one arm in `candidate_root`.
//! - Lifecycle: one per daemon; the cache is its only mutable state, behind a
//!   mutex.
//! - Errors: `Verified { index, root }` vs `Unverified { per_candidate }`; an
//!   unreachable host counts as unverified (and is not cached).
//! - Role: domain service (stateless apart from a bounded verdict cache).

use crate::{
    host::hc::foreign_terminal_process_names,
    ports::{HostCallContext, HostPort, ProcessInfo},
    protocol::{
        ids::HostTargetId,
        locate::{CandidateVerdict, LocateCaller, LocateCandidate, LocateReply},
        results::{ApiError, ErrorCode},
        time::CallBudget,
    },
};
use std::{
    collections::HashMap,
    ffi::OsString,
    sync::{Arc, Mutex},
};

/// One process environment: `(name, value)` pairs in the order the process holds them.
pub type Environ = Vec<(OsString, OsString)>;

/// The environment of process `pid`, when this user may read it; `None` on any failure (another
/// user's process, a gone pid, an unsupported platform). The caller skips its check on `None`.
pub fn read_server_environ(pid: u32) -> Option<Environ> {
    platform_environ(pid)
}

#[cfg(target_os = "linux")]
fn platform_environ(pid: u32) -> Option<Environ> {
    let raw = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    Some(parse_environ_block(&raw))
}

/// `KERN_PROCARGS2` of `pid`: argc, the executable path, argv, then the environment.
#[cfg(target_os = "macos")]
fn platform_environ(pid: u32) -> Option<Environ> {
    let pid = libc::c_int::try_from(pid).ok()?;
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    // SAFETY: a size query with a null buffer, then a read into a buffer of the size reported.
    unsafe {
        let mut size: libc::size_t = 0;
        if libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
            || size == 0
        {
            return None;
        }
        let mut buffer = vec![0_u8; size];
        if libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buffer.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return None;
        }
        buffer.truncate(size);
        parse_procargs2(&buffer)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn platform_environ(_pid: u32) -> Option<Environ> {
    None
}

/// A NUL-separated `NAME=value` block (Linux `/proc/<pid>/environ`); an entry with no `=` is
/// dropped.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn parse_environ_block(raw: &[u8]) -> Environ {
    raw.split(|b| *b == 0)
        .filter_map(|entry| {
            let eq = entry.iter().position(|b| *b == b'=')?;
            Some((bytes_to_os(&entry[..eq]), bytes_to_os(&entry[eq + 1..])))
        })
        .collect()
}

/// The macOS `KERN_PROCARGS2` buffer: a native-endian `int` argc, the executable path and its NUL
/// padding, argc argv strings, then the environment strings up to the first empty one. Plain Rust
/// on every OS so a captured buffer is testable anywhere; `None` when it is truncated or malformed.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn parse_procargs2(buffer: &[u8]) -> Option<Environ> {
    let argc = i32::from_ne_bytes(buffer.get(..4)?.try_into().ok()?);
    let argc = usize::try_from(argc).ok()?;
    let mut rest = buffer.get(4..)?;
    // The executable path, then the NUL padding that follows it.
    let path_end = rest.iter().position(|b| *b == 0)?;
    rest = &rest[path_end..];
    let pad = rest.iter().position(|b| *b != 0)?;
    rest = &rest[pad..];
    // argv: argc NUL-terminated strings.
    for _ in 0..argc {
        let end = rest.iter().position(|b| *b == 0)?;
        rest = &rest[end + 1..];
    }
    let mut environ = Environ::new();
    while let Some(end) = rest.iter().position(|b| *b == 0) {
        let entry = &rest[..end];
        if entry.is_empty() {
            break;
        }
        if let Some(eq) = entry.iter().position(|b| *b == b'=') {
            environ.push((bytes_to_os(&entry[..eq]), bytes_to_os(&entry[eq + 1..])));
        }
        rest = &rest[end + 1..];
    }
    Some(environ)
}

fn bytes_to_os(bytes: &[u8]) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(bytes.to_vec())
}

/// M6 (129 bytes + the name and the host label), after the CLI's `herdr-threads: ` prefix.
pub fn inherited_name_detail(name: &str, host: &str) -> String {
    format!(
        "{}={name} inherited from the {host} server; use another name per command or restart the server without it",
        crate::host::named::ENV_SEAT_NAME
    )
}

/// The core a terminal candidate names, by socket: attached, or attached now.
pub type CorePort<'a> = &'a dyn Fn(&str) -> Result<Arc<dyn HostPort>, ApiError>;

/// The candidates of a `ResolveSeat.locator_hint`: parts joined by `\n`; a part starting with `/`
/// is `<socket>:<tN>`, split at its last `:`; anything else is a pane.
pub fn hint_candidates(hint: &str) -> Vec<LocateCandidate> {
    hint.split('\n')
        .filter(|part| !part.is_empty())
        .map(
            |part| match part.strip_prefix('/').and(part.rsplit_once(':')) {
                Some((socket, id)) => LocateCandidate::Terminal {
                    socket: socket.to_owned(),
                    id: id.to_owned(),
                },
                None => LocateCandidate::Pane {
                    id: part.to_owned(),
                },
            },
        )
        .collect()
}

/// The inherited-name check (SA2, D1): whether the server that owns a hinted pane or terminal
/// carries `name` in its own environment, read through `read`.
pub fn inherited_name(
    name: &str,
    hint: &str,
    hosts: &[Arc<dyn HostPort>],
    core: CorePort<'_>,
    context: &HostCallContext,
    read: impl Fn(u32) -> Option<Environ>,
) -> bool {
    inherited_name_host(name, hint, hosts, core, context, read).is_some()
}

/// [`inherited_name`] naming (`HostPort::server_label`) the host whose server holds the name,
/// for the first candidate of the hint whose server does. A pane candidate asks the hosts that
/// do not answer terminal ids; a terminal candidate asks only the core its socket names
/// (`core`, attached on a miss), and a core that cannot be had skips that candidate. A server
/// with no pid, or one whose environment is unreadable, skips the check (D1 limit): it never
/// refuses on a read that failed. The comparison is on the name alone, so a per-command name
/// equal to the server's own is refused too (an accepted limit).
pub fn inherited_name_host(
    name: &str,
    hint: &str,
    hosts: &[Arc<dyn HostPort>],
    core: CorePort<'_>,
    context: &HostCallContext,
    read: impl Fn(u32) -> Option<Environ>,
) -> Option<&'static str> {
    let holds = |host: &Arc<dyn HostPort>, target: &HostTargetId| {
        matches!(host.target_root_process(target, context), Ok(Some(_)))
            && host
                .server_process()
                .and_then(&read)
                .is_some_and(|environ| {
                    environ.iter().any(|(key, value)| {
                        key.as_os_str() == crate::host::named::ENV_SEAT_NAME
                            && value.as_os_str() == name
                    })
                })
    };
    hint_candidates(hint)
        .into_iter()
        .find_map(|candidate| match candidate {
            LocateCandidate::Pane { id } => {
                let target = HostTargetId::parse(id).ok()?;
                hosts
                    .iter()
                    .filter(|host| !host.answers_terminal_ids())
                    .find(|host| holds(host, &target))
                    .map(|host| host.server_label())
            }
            LocateCandidate::Terminal { socket, id } => {
                let target = HostTargetId::parse(id).ok()?;
                let port = core(&socket).ok()?;
                holds(&port, &target).then(|| port.server_label())
            }
        })
}

/// Longest ancestry the walk follows (a loop or a pathological tree ends it).
const MAX_WALK: usize = 128;
/// Verdicts kept; past it the cache starts over.
const MAX_CACHED: usize = 256;

type CacheKey = (Vec<LocateCandidate>, u32, u64, u64);

/// One pid of the walk and whether it is a herdr-core terminal process (a wall).
struct Ancestor {
    pid: u32,
    wall: bool,
}

pub struct LocateCallerService {
    processes: Arc<dyn ProcessInfo>,
    cache: Mutex<HashMap<CacheKey, LocateReply>>,
}

/// What asking the hosts about one candidate gave.
enum Root {
    Pid(u32),
    Verdict(CandidateVerdict),
}

impl LocateCallerService {
    pub fn new(processes: Arc<dyn ProcessInfo>) -> Self {
        Self {
            processes,
            cache: Mutex::default(),
        }
    }

    /// Verifies `request` for a caller whose connection came from `peer_pid`.
    /// `hosts` are the ports of `HostSessions::all_for_reads`, `core` the port of a terminal
    /// candidate's socket; `epoch` names
    /// that set (it changes when a host attaches or restarts, dropping every
    /// cached verdict).
    pub fn locate(
        &self,
        request: &LocateCaller,
        peer_pid: Option<u32>,
        hosts: &[Arc<dyn HostPort>],
        core: CorePort<'_>,
        epoch: u64,
        budget: &CallBudget,
    ) -> LocateReply {
        self.verdict(request, peer_pid, hosts, core, epoch, budget)
    }

    fn verdict(
        &self,
        request: &LocateCaller,
        peer_pid: Option<u32>,
        hosts: &[Arc<dyn HostPort>],
        core: CorePort<'_>,
        epoch: u64,
        budget: &CallBudget,
    ) -> LocateReply {
        let unverified = |verdict| LocateReply::Unverified {
            per_candidate: vec![verdict; request.candidates.len()],
        };
        let Some(subject) = request.subject.map(|f| f.pid).or(peer_pid) else {
            return unverified(CandidateVerdict::NotAncestor);
        };
        // The start stamp keeps a reused pid off another process's verdict; a subject whose
        // record is unreadable has none, so it is never cached (its walk is cheap).
        let key = self
            .processes
            .process(subject)
            .map(|r| (request.candidates.clone(), subject, r.start, epoch));
        if let Some(hit) = key.as_ref().and_then(|key| {
            self.cache
                .lock()
                .ok()
                .and_then(|cache| cache.get(key).cloned())
        }) {
            return hit;
        }
        let ancestors = self.ancestry(subject);
        let context = HostCallContext {
            budget: budget.clone(),
            expected_boot: None,
            expected_epoch: None,
        };
        let roots: Vec<Root> = request
            .candidates
            .iter()
            .map(|candidate| candidate_root(candidate, hosts, core, &context))
            .collect();
        let reply = verify(&ancestors, &roots);
        let transient = matches!(&reply, LocateReply::Unverified { per_candidate, .. }
            if per_candidate.contains(&CandidateVerdict::Unreachable));
        if !transient
            && let Some(key) = key
            && let Ok(mut cache) = self.cache.lock()
        {
            if cache.len() >= MAX_CACHED {
                cache.clear();
            }
            cache.insert(key, reply.clone());
        }
        reply
    }

    /// `subject` and its parents, nearest first, each marked when it is a wall.
    fn ancestry(&self, subject: u32) -> Vec<Ancestor> {
        let mut chain: Vec<Ancestor> = Vec::new();
        let mut next = Some(subject);
        while let Some(pid) = next {
            if (pid == 0 && !chain.is_empty())
                || chain.iter().any(|a| a.pid == pid)
                || chain.len() >= MAX_WALK
            {
                break;
            }
            let record = self.processes.process(pid);
            let wall = record.as_ref().is_some_and(|r| {
                r.argv.first().is_some_and(|argv0| {
                    let name = argv0.rsplit('/').next().unwrap_or(argv0);
                    foreign_terminal_process_names().contains(&name)
                })
            });
            chain.push(Ancestor { pid, wall });
            next = record.and_then(|r| r.parent);
        }
        chain
    }
}

/// Nearest ancestor first: the first pid on the chain that is some candidate's root wins.
/// A wall process may itself be a root, but nothing above it verifies.
fn verify(ancestors: &[Ancestor], roots: &[Root]) -> LocateReply {
    for Ancestor { pid, wall } in ancestors {
        if let Some(index) = roots
            .iter()
            .position(|root| matches!(root, Root::Pid(root) if root == pid))
        {
            return LocateReply::Verified { index, root: *pid };
        }
        if *wall {
            break;
        }
    }
    LocateReply::Unverified {
        per_candidate: roots
            .iter()
            .map(|root| match root {
                Root::Pid(_) => CandidateVerdict::NotAncestor,
                Root::Verdict(verdict) => *verdict,
            })
            .collect(),
    }
}

fn candidate_root(
    candidate: &LocateCandidate,
    hosts: &[Arc<dyn HostPort>],
    core: CorePort<'_>,
    context: &HostCallContext,
) -> Root {
    match candidate {
        // A pane id means nothing to a core: only the hosts that do not answer terminal ids.
        LocateCandidate::Pane { id } => {
            let Ok(target) = HostTargetId::parse(id.clone()) else {
                return Root::Verdict(CandidateVerdict::NoRoot);
            };
            let mut verdict = CandidateVerdict::Unreachable;
            for host in hosts.iter().filter(|host| !host.answers_terminal_ids()) {
                match host.target_root_process(&target, context) {
                    Ok(Some(pid)) => return Root::Pid(pid),
                    Ok(None) => verdict = CandidateVerdict::NoRoot,
                    Err(_) => {}
                }
            }
            Root::Verdict(verdict)
        }
        // A terminal id is local to the core its socket names, so only that core is asked.
        LocateCandidate::Terminal { socket, id } => {
            let Ok(target) = HostTargetId::parse(id.clone()) else {
                return Root::Verdict(CandidateVerdict::NoRoot);
            };
            match core(socket) {
                Ok(port) => match port.target_root_process(&target, context) {
                    Ok(Some(pid)) => Root::Pid(pid),
                    Ok(None) => Root::Verdict(CandidateVerdict::NoRoot),
                    Err(_) => Root::Verdict(CandidateVerdict::Unreachable),
                },
                Err(error) if error.code == ErrorCode::Unsupported => {
                    Root::Verdict(CandidateVerdict::Unsupported)
                }
                Err(_) => Root::Verdict(CandidateVerdict::Unreachable),
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/service/locate.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "../../tests/service/environ_reader.rs"]
pub(crate) mod environ_reader_tests;
