//! Remote launch over the peer link (`peer.launch.v1`, TRUST-POLICY N15).
//!
//! Responsibility (remote_launch)
//! - Owns: this node's outstanding `launch_request`s (by id, with their link
//!   and reply channel), the bound on a peer's launch requests in flight
//!   here, and the hand-off of an inbound request to the daemon's
//!   [`PeerLauncher`].
//! - Hides: request ids and the link a request travels on.
//! - Must not: launch, record or decide anything itself: the receiving
//!   node's launcher decides against its own canonical view and records the
//!   request (`peer_requests`) before acting.
//! - Role: fed by `SyncHandle::launch_remote` and by `on_frame`; a link's end
//!   fails its requests; housekeeping drops expired ones.
//!
//! Core and harness-neutral: the harness id is passed through as text.

use std::sync::OnceLock;

use super::*;
use crate::sync::frame::{
    LaunchReply, LaunchRequest, LaunchState, LaunchTarget, MAX_LAUNCH_ARGS, MAX_LAUNCH_TEXT_BYTES,
    PEER_LAUNCH_CAP,
};

/// Launch requests one peer may have in flight here (answering is quick:
/// the launcher records and starts, it never waits for the launch).
pub const MAX_INBOUND_IN_FLIGHT: usize = 8;
/// Requests this node may have outstanding, across all links.
const MAX_OUTSTANDING: usize = 64;
/// An outstanding request unanswered after this long is dropped.
const OUTSTANDING_TTL_MS: u64 = 60_000;

/// Decides and starts (or reports) a launch a linked peer asked for. Called
/// on a blocking thread; returns without waiting for the launch.
pub trait PeerLauncher: Send + Sync {
    fn launch(&self, from: NodeUuid, request: LaunchRequest) -> LaunchState;
}

/// Where the daemon's launcher is installed once its hosts exist.
pub type LauncherSlot = Arc<OnceLock<Arc<dyn PeerLauncher>>>;

/// What became of one outbound launch request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchAnswer {
    Answered(LaunchState),
    /// No established active link to the node.
    NotLinked,
    /// The link does not carry [`PEER_LAUNCH_CAP`]; the peer's version.
    NoCap {
        version: Option<String>,
    },
    /// The link ended before the answer.
    LinkLost,
}

pub(super) struct Outstanding {
    link_id: u64,
    at: MonoInstant,
    reply: oneshot::Sender<LaunchAnswer>,
}

pub(super) struct Served {
    link_id: u64,
    id: Uuid,
    state: LaunchState,
}

pub(super) struct RemoteLaunchState {
    pub(super) launcher: LauncherSlot,
    outstanding: BTreeMap<Uuid, Outstanding>,
    in_flight: BTreeMap<u64, usize>,
    pub(super) serving: JoinSet<Served>,
}

impl RemoteLaunchState {
    pub(super) fn new(launcher: LauncherSlot) -> Self {
        Self {
            launcher,
            outstanding: BTreeMap::new(),
            in_flight: BTreeMap::new(),
            serving: JoinSet::new(),
        }
    }
}

fn bounded(text: &str) -> String {
    let mut end = text.len().min(MAX_LAUNCH_TEXT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Why `request` cannot be taken as sent, if it cannot.
fn malformed(request: &LaunchRequest) -> Option<&'static str> {
    let long = |t: &str| t.len() > MAX_LAUNCH_TEXT_BYTES;
    let plan = &request.plan;
    let target = match &plan.target {
        LaunchTarget::Pane { space, tab, pane } => {
            pane.is_empty()
                || long(pane)
                || space.as_deref().is_some_and(long)
                || tab.as_deref().is_some_and(long)
        }
        LaunchTarget::NewTab { space, label } => {
            space.is_empty() || label.trim().is_empty() || long(space) || long(label)
        }
        LaunchTarget::Unknown => return Some("unsupported launch target"),
    };
    if target
        || plan.kind.is_empty()
        || long(&plan.kind)
        || long(&request.requester_seat)
        || plan.argv.len() > MAX_LAUNCH_ARGS
        || plan.argv.iter().any(|a| long(a))
        || plan.cwd.as_deref().is_some_and(long)
    {
        return Some("launch request out of bounds");
    }
    None
}

fn refused(detail: &str) -> LaunchState {
    LaunchState::Refused {
        detail: bounded(detail),
    }
}

impl Actor {
    fn launch_link(&self, node: &NodeUuid) -> Option<u64> {
        self.by_node.get(node)?.iter().copied().find(|id| {
            self.links
                .get(id)
                .is_some_and(|l| l.mode == LinkMode::Active && l.ranked && !l.closing)
        })
    }

    /// Send `request` to `node` and answer `reply` when it answers.
    pub(super) fn on_launch_remote(
        &mut self,
        node: NodeUuid,
        request: LaunchRequest,
        reply: oneshot::Sender<LaunchAnswer>,
    ) {
        let Some(link_id) = self.launch_link(&node) else {
            let _ = reply.send(LaunchAnswer::NotLinked);
            return;
        };
        let entry = &self.links[&link_id];
        if !entry
            .handle
            .peer()
            .caps
            .iter()
            .any(|c| c == PEER_LAUNCH_CAP)
        {
            let _ = reply.send(LaunchAnswer::NoCap {
                version: entry.peer_version.clone(),
            });
            return;
        }
        if self.launch.outstanding.len() >= MAX_OUTSTANDING
            || self.launch.outstanding.contains_key(&request.id)
        {
            let _ = reply.send(LaunchAnswer::Answered(LaunchState::Unavailable {
                reason: "too many launch requests in flight on the asking node".into(),
            }));
            return;
        }
        let id = request.id;
        if entry
            .handle
            .send(PeerFrame::LaunchRequest(request))
            .is_err()
        {
            let _ = reply.send(LaunchAnswer::LinkLost);
            return;
        }
        let at = self.mono();
        self.launch
            .outstanding
            .insert(id, Outstanding { link_id, at, reply });
    }

    /// A `launch_request` from a peer: answered by the launcher on a blocking
    /// thread, or at once when it cannot be.
    pub(super) fn on_launch_request(&mut self, link_id: u64, request: LaunchRequest) {
        let Some(entry) = self.links.get(&link_id) else {
            return;
        };
        if !entry
            .handle
            .peer()
            .caps
            .iter()
            .any(|c| c == PEER_LAUNCH_CAP)
        {
            return;
        }
        let from = entry.handle.peer().node;
        let answer_now = |state: LaunchState| {
            let _ = entry.handle.send(PeerFrame::LaunchReply(LaunchReply {
                id: request.id,
                state,
            }));
        };
        if let Some(why) = malformed(&request) {
            return answer_now(refused(why));
        }
        let Some(launcher) = self.launch.launcher.get().cloned() else {
            return answer_now(LaunchState::Unavailable {
                reason: "launcher unavailable".into(),
            });
        };
        let busy = self.launch.in_flight.entry(link_id).or_default();
        if *busy >= MAX_INBOUND_IN_FLIGHT {
            return answer_now(LaunchState::Unavailable {
                reason: "busy".into(),
            });
        }
        *busy += 1;
        let id = request.id;
        self.launch.serving.spawn(async move {
            let state = tokio::task::spawn_blocking(move || launcher.launch(from, request))
                .await
                .unwrap_or_else(|_| LaunchState::Unavailable {
                    reason: "launcher failed".into(),
                });
            Served { link_id, id, state }
        });
    }

    pub(super) fn on_launch_served(&mut self, joined: Result<Served, tokio::task::JoinError>) {
        let Ok(served) = joined else {
            return;
        };
        if let Some(busy) = self.launch.in_flight.get_mut(&served.link_id) {
            *busy = busy.saturating_sub(1);
            if *busy == 0 {
                self.launch.in_flight.remove(&served.link_id);
            }
        }
        if let Some(entry) = self.links.get(&served.link_id) {
            let _ = entry.handle.send(PeerFrame::LaunchReply(LaunchReply {
                id: served.id,
                state: bound_state(served.state),
            }));
        }
    }

    /// A `launch_reply`: completes the request of that id, only from the link
    /// it went out on.
    pub(super) fn on_launch_reply(&mut self, link_id: u64, reply: LaunchReply) {
        if self
            .launch
            .outstanding
            .get(&reply.id)
            .is_some_and(|o| o.link_id == link_id)
            && let Some(done) = self.launch.outstanding.remove(&reply.id)
        {
            let _ = done.reply.send(LaunchAnswer::Answered(reply.state));
        }
    }

    pub(super) fn launch_link_ended(&mut self, link_id: u64) {
        self.launch.in_flight.remove(&link_id);
        let lost: Vec<Uuid> = self
            .launch
            .outstanding
            .iter()
            .filter(|(_, o)| o.link_id == link_id)
            .map(|(id, _)| *id)
            .collect();
        for id in lost {
            if let Some(done) = self.launch.outstanding.remove(&id) {
                let _ = done.reply.send(LaunchAnswer::LinkLost);
            }
        }
    }

    pub(super) fn launch_housekeeping(&mut self) {
        let now = self.mono();
        self.launch.outstanding.retain(|_, o| {
            !o.reply.is_closed() && now.0.saturating_sub(o.at.0) <= OUTSTANDING_TTL_MS
        });
    }
}

/// Every text of `state` bounded before it is sent.
fn bound_state(state: LaunchState) -> LaunchState {
    match state {
        LaunchState::Running { pane } => LaunchState::Running {
            pane: pane.map(|p| bounded(&p)),
        },
        LaunchState::Launched { seat, pane } => LaunchState::Launched {
            seat: bounded(&seat),
            pane: bounded(&pane),
        },
        LaunchState::Refused { detail } => refused(&detail),
        LaunchState::Failed {
            detail,
            pane,
            unconfirmed,
        } => LaunchState::Failed {
            detail: bounded(&detail),
            pane: pane.map(|p| bounded(&p)),
            unconfirmed,
        },
        LaunchState::Unavailable { reason } => LaunchState::Unavailable {
            reason: bounded(&reason),
        },
        LaunchState::Unknown => LaunchState::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::frame::LaunchPlan;

    fn request(target: LaunchTarget) -> LaunchRequest {
        LaunchRequest {
            id: Uuid::from_u128(1),
            requester_seat: "sQ0Vseat0001".into(),
            plan: LaunchPlan {
                target,
                kind: "codex".into(),
                argv: vec!["--model".into(), "x".into()],
                cwd: Some("/tmp".into()),
            },
        }
    }

    #[test]
    fn malformed_requests_are_named() {
        assert_eq!(
            malformed(&request(LaunchTarget::Pane {
                space: None,
                tab: None,
                pane: "w1:p1".into()
            })),
            None
        );
        assert_eq!(
            malformed(&request(LaunchTarget::Unknown)),
            Some("unsupported launch target")
        );
        assert!(
            malformed(&request(LaunchTarget::NewTab {
                space: "w1".into(),
                label: " ".into()
            }))
            .is_some()
        );
        let mut long = request(LaunchTarget::NewTab {
            space: "w1".into(),
            label: "review".into(),
        });
        long.plan.argv = vec!["a".into(); MAX_LAUNCH_ARGS + 1];
        assert!(malformed(&long).is_some());
    }
}
