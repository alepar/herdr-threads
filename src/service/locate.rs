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
//!   uncached; the server-environ read for the inherited-name refusal (M6)
//!   lands with the named-seat phase.
//! - Hides: how a pid maps to a host target.
//! - Must not: run for a `One` caller (that would be adversarial verification,
//!   which TRUST-POLICY forbids; the CLI sends this command only for
//!   `Ambiguous`), fall back to a candidate when none verifies, or fail closed
//!   on an unreadable server environ (`server_process() == None` skips that
//!   check).
//! - Relationships: delegates to `HostPort::target_root_process` of the
//!   sessions `HostSessions::all_for_reads` returns. A terminal candidate asks the
//!   hosts that answer terminal ids (`answers_terminal_ids`), a pane candidate the
//!   others; a terminal no attached host answers for is `Unsupported`. With both
//!   families the nearest ancestor wins, whichever family owns it.
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
        time::CallBudget,
    },
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

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
    /// `hosts` are the ports of `HostSessions::all_for_reads`; `epoch` names
    /// that set (it changes when a host attaches or restarts, dropping every
    /// cached verdict).
    pub fn locate(
        &self,
        request: &LocateCaller,
        peer_pid: Option<u32>,
        hosts: &[Arc<dyn HostPort>],
        epoch: u64,
        budget: &CallBudget,
    ) -> LocateReply {
        self.verdict(request, peer_pid, hosts, epoch, budget)
    }

    fn verdict(
        &self,
        request: &LocateCaller,
        peer_pid: Option<u32>,
        hosts: &[Arc<dyn HostPort>],
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
            .map(|candidate| candidate_root(candidate, hosts, &context))
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
    context: &HostCallContext,
) -> Root {
    let (target, terminal_family) = match candidate {
        LocateCandidate::Pane { id } => (id, false),
        LocateCandidate::Terminal { id, .. } => (id, true),
    };
    let Ok(target) = HostTargetId::parse(target.clone()) else {
        return Root::Verdict(CandidateVerdict::NoRoot);
    };
    // A family asks only the hosts of its own kind: a pane id means nothing to a core, nor a
    // terminal id to a Herdr server.
    let mut asked = false;
    let mut verdict = CandidateVerdict::Unreachable;
    for host in hosts
        .iter()
        .filter(|host| host.answers_terminal_ids() == terminal_family)
    {
        asked = true;
        match host.target_root_process(&target, context) {
            Ok(Some(pid)) => return Root::Pid(pid),
            Ok(None) => verdict = CandidateVerdict::NoRoot,
            Err(_) => {}
        }
    }
    if !asked && terminal_family {
        return Root::Verdict(CandidateVerdict::Unsupported);
    }
    Root::Verdict(verdict)
}

#[cfg(test)]
#[path = "../../tests/service/locate.rs"]
pub(crate) mod tests;
