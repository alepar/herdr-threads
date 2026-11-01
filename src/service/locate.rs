//! `LocateCaller`: daemon-side nearest-ancestor verification of a two-family caller.
//!
//! Responsibility (LocateCaller)
//! - Owns: the walk from the subject (the claim's liveness pid, else the
//!   transport peer pid) up `ProcessInfo` parents against each candidate's
//!   `HostPort::target_root_process`, and its cache keyed on (candidates,
//!   subject pid, host set). The terminal-root wall and the start-stamped key
//!   (unstamped subjects uncached) land with ht-kqrt.74; the server-environ
//!   read for the inherited-name refusal (M6) lands with the named-seat phase.
//! - Hides: how a pid maps to a host target.
//! - Must not: run for a `One` caller (that would be adversarial verification,
//!   which TRUST-POLICY forbids; the CLI sends this command only for
//!   `Ambiguous`), fall back to a candidate when none verifies, or fail closed
//!   on an unreadable server environ (`server_process() == None` skips that
//!   check).
//! - Relationships: delegates to `HostPort::target_root_process` of the
//!   sessions `HostSessions::all_for_reads` returns; it knows no host kind and
//!   matches only on the candidate's family. A candidate family no host
//!   answers for (a herdr-core terminal, until H1) is `Unsupported`.
//! - Where new behaviour goes: a new host candidate verifies through its host's
//!   `target_root_process` answer, with one arm in `candidate_root`.
//! - Lifecycle: one per daemon; the cache is its only mutable state, behind a
//!   mutex. The per-binding guard memory (`guard_key`) is its second.
//! - Errors: `Verified { index, root }` vs `Unverified { per_candidate }`; an
//!   unreachable host counts as unverified (and is not cached).
//! - Role: domain service (stateless apart from a bounded verdict cache).

use crate::{
    ports::{HostCallContext, HostPort, ProcessInfo},
    protocol::{
        ids::HostTargetId,
        locate::{CandidateVerdict, LocateCaller, LocateCandidate, LocateReply},
        time::CallBudget,
    },
};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

/// Longest ancestry the walk follows (a loop or a pathological tree ends it).
const MAX_WALK: usize = 128;
/// Verdicts kept; past it the cache starts over.
const MAX_CACHED: usize = 256;

type CacheKey = (Vec<LocateCandidate>, u32, u64);

pub struct LocateCallerService {
    processes: Arc<dyn ProcessInfo>,
    cache: Mutex<HashMap<CacheKey, LocateReply>>,
    /// Guard keys already answered `Unverified` (the per-binding M3 memory).
    guarded: Mutex<HashSet<String>>,
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
            guarded: Mutex::default(),
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
        let reply = self.verdict(request, peer_pid, hosts, epoch, budget);
        match (reply, &request.guard_key) {
            (LocateReply::Unverified { per_candidate, .. }, Some(key)) => LocateReply::Unverified {
                per_candidate,
                first_guard: self.first_guard(key),
            },
            (reply, _) => reply,
        }
    }

    /// Remembers `key`; true the first time this daemon sees it (memory only, so a
    /// restart arms it again; bounded, past it the set starts over).
    fn first_guard(&self, key: &str) -> bool {
        let Ok(mut seen) = self.guarded.lock() else {
            return false;
        };
        if seen.len() >= MAX_CACHED * 16 {
            seen.clear();
        }
        seen.insert(key.to_owned())
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
            first_guard: false,
        };
        let Some(subject) = request.subject.map(|f| f.pid).or(peer_pid) else {
            return unverified(CandidateVerdict::NotAncestor);
        };
        let key = (request.candidates.clone(), subject, epoch);
        if let Some(hit) = self.cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
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
        if !transient && let Ok(mut cache) = self.cache.lock() {
            if cache.len() >= MAX_CACHED {
                cache.clear();
            }
            cache.insert(key, reply.clone());
        }
        reply
    }

    /// `subject` and its parents, nearest first.
    fn ancestry(&self, subject: u32) -> Vec<u32> {
        let mut chain = vec![subject];
        let mut next = self.processes.process(subject).and_then(|r| r.parent);
        while let Some(pid) = next {
            if pid == 0 || chain.contains(&pid) || chain.len() >= MAX_WALK {
                break;
            }
            chain.push(pid);
            next = self.processes.process(pid).and_then(|r| r.parent);
        }
        chain
    }
}

/// Nearest ancestor first: the first pid on the chain that is some candidate's root wins.
fn verify(ancestors: &[u32], roots: &[Root]) -> LocateReply {
    for pid in ancestors {
        if let Some(index) = roots
            .iter()
            .position(|root| matches!(root, Root::Pid(root) if root == pid))
        {
            return LocateReply::Verified { index, root: *pid };
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
        first_guard: false,
    }
}

fn candidate_root(
    candidate: &LocateCandidate,
    hosts: &[Arc<dyn HostPort>],
    context: &HostCallContext,
) -> Root {
    match candidate {
        LocateCandidate::Pane { id } => {
            let Ok(target) = HostTargetId::parse(id.clone()) else {
                return Root::Verdict(CandidateVerdict::NoRoot);
            };
            let mut verdict = CandidateVerdict::Unreachable;
            for host in hosts {
                match host.target_root_process(&target, context) {
                    Ok(Some(pid)) => return Root::Pid(pid),
                    Ok(None) => verdict = CandidateVerdict::NoRoot,
                    Err(_) => {}
                }
            }
            Root::Verdict(verdict)
        }
        // No host answers for a terminal until H1 builds one.
        LocateCandidate::Terminal { .. } => Root::Verdict(CandidateVerdict::Unsupported),
    }
}

#[cfg(test)]
#[path = "../../tests/service/locate.rs"]
pub(crate) mod tests;
