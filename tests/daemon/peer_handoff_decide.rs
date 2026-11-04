//! `PeerLaunchService::decide` for a handoff plan (spec 3.1): each refusal in order, recorded
//! `refused` with no journal entry; an unreplicated sender answered `Unavailable` and not
//! recorded; the frozen prompt; a repeated id answered from its row. The placement is the
//! consumer audit's dialled fake (a new tab on a Herdr-shaped host).

use super::consumer_audit::{AuditHost, Rig};
use crate::{
    cli::handoff::summary_prompt::{SenderLine, render_v2},
    node::{NodeIdentity, NodeUuid},
    protocol::ids::{MessageId, SeatId},
    protocol::summary::AuthorRole,
    service::peer_launcher::{PeerLaunchService, handoff_directory},
    store::peer_requests,
    sync::coordinator::remote_launch::PeerLauncher,
    sync::frame::{LaunchPlan, LaunchRequest, LaunchState, LaunchTarget},
    sync::handoff_block::SenderBlock,
};
use rusqlite::params;
use std::sync::Arc;
use uuid::Uuid;

const SEAT: &str = "sAAAAAAAAAAA";
const SUMMARY: &str = "review the parser";

struct Fx {
    rig: Rig,
    launcher: PeerLaunchService,
    identity: NodeIdentity,
    cwd: std::path::PathBuf,
    profile: std::path::PathBuf,
    kind: &'static str,
}

/// A registered harness whose adapter projects a profile directory and takes startup input.
fn handoff_kind(profile: &std::path::Path) -> &'static str {
    let registry = crate::harness::registry::builtins();
    registry
        .registrations()
        .iter()
        .map(|r| r.metadata().id)
        .find(|id| {
            let plan = LaunchPlan {
                kind: (*id).into(),
                profile_dir: Some(profile.to_str().unwrap().into()),
                ..LaunchPlan::default()
            };
            registry.agent(id).is_ok() && matches!(handoff_directory(&plan), Ok(Some(_)))
        })
        .expect("a harness that takes an explicit profile directory")
}

impl Fx {
    fn new(label: &str) -> Self {
        let rig = Rig::new(label);
        rig.dial("/herdr/a.sock", AuditHost::HERDR.with_space("w1"));
        let launcher = PeerLaunchService {
            live: Default::default(),
            store: Arc::clone(&rig.store),
            sessions: Arc::clone(&rig.sessions),
            writer: Arc::new(crate::service::fair_writer::FairWriter::new(32)),
            clock: Arc::clone(&rig.clock),
            state_dir: std::path::PathBuf::from("/nonexistent"),
            log: Arc::new(|_| {}),
        };
        let cwd = rig.dir("cwd");
        let profile = rig.dir("profile");
        let kind = handoff_kind(&profile);
        Self {
            rig,
            launcher,
            identity: NodeIdentity::from_seed([3; 32]),
            cwd,
            profile,
            kind,
        }
    }

    fn from(&self) -> NodeUuid {
        self.identity.uuid()
    }

    /// The sender node's log row (its key) as the journal would hold it.
    fn replicate_node(&self) {
        self.rig
            .db()
            .execute(
                "INSERT INTO sync_logs(node_uuid,incarnation,is_self,pubkey,genesis_hash,contiguous_counter,head_hash,first_seen_at) \
                 VALUES (?1,1,0,?2,zeroblob(32),0,zeroblob(32),0)",
                params![self.from().to_string(), self.identity.public_key().as_slice()],
            )
            .unwrap();
    }

    /// The sender seat as the directory holds a remote seat homed on `home`.
    fn replicate_seat(&self, home: &str, name: Option<&str>) {
        self.rig
            .db()
            .execute(
                "INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,remote_kind,host_id,published,name) \
                 VALUES (?1,(SELECT id FROM local_node),'remote','native',0,0,?2,0,'agent',NULL,1,?3)",
                params![SEAT, home, name],
            )
            .unwrap();
    }

    fn block(&self, id: Uuid) -> SenderBlock {
        let mut block = SenderBlock {
            v: 1,
            request: id,
            sender_node: Some(self.from()),
            author: Some(SeatId::new(SEAT)),
            author_role: Some(AuthorRole::Agent),
            message: Some(MessageId::new("mHOFFabcdefgh")),
            body: "do the thing".into(),
            priority: 0,
            require_ack: true,
            requested_at: 1_700_000_000_000,
            ack_after_ms: Some(600_000),
            sig: String::new(),
        };
        block.sign(&self.identity).unwrap();
        block
    }

    fn request(&self, n: u128) -> LaunchRequest {
        let id = Uuid::from_u128(n);
        LaunchRequest {
            id,
            requester_seat: SEAT.into(),
            plan: LaunchPlan {
                target: LaunchTarget::NewTab {
                    space: "w1".into(),
                    label: "t".into(),
                },
                kind: self.kind.into(),
                argv: Vec::new(),
                cwd: Some(self.cwd.to_str().unwrap().into()),
                profile_dir: Some(self.profile.to_str().unwrap().into()),
                handoff_prompt: Some(SUMMARY.into()),
                message: Some(Box::new(self.block(id))),
            },
        }
    }

    fn journal_entries(&self) -> i64 {
        self.rig
            .db()
            .query_row("SELECT count(*) FROM journal", [], |r| r.get(0))
            .unwrap()
    }

    fn row(&self, request: &LaunchRequest) -> Option<peer_requests::PeerRequest> {
        peer_requests::get(&self.rig.db(), &request.id.to_string()).unwrap()
    }

    /// `request` must be refused with `detail` (of `len` bytes), recorded `refused` with that
    /// detail, and leave the journal as it was.
    fn assert_refused(&self, request: &LaunchRequest, detail: &str, len: usize) {
        let entries = self.journal_entries();
        let state = self.launcher.launch(self.from(), request.clone());
        assert_eq!(
            state,
            LaunchState::Refused {
                detail: detail.into()
            }
        );
        assert_eq!(detail.len(), len, "the stated byte size");
        let row = self.row(request).expect("a refusal is recorded");
        assert_eq!(
            (row.state.as_str(), row.detail.as_deref()),
            ("refused", Some(detail))
        );
        assert_eq!(row.handoff_prompt, None, "a refusal freezes no prompt");
        assert_eq!(self.journal_entries(), entries, "a refusal writes no fact");
    }
}

const UNVERIFIED: &str = "handoff message does not verify against its sender node";

// Kills: a handoff check that is skipped or reordered: each of the sender node, author, request
// id and the signature refused with the 55-byte detail, the seat's home with the 44-byte one,
// a retired seat with the 31-byte one, and an unusable profile directory with the 60-byte one; and an earlier check masked by a
// later one (a request wrong in several ways answers the first).
#[test]
fn refusals_in_order_leave_a_refused_row_and_no_journal_entry() {
    let fx = Fx::new("handoff-decide-order");
    fx.replicate_node();
    fx.replicate_seat(&fx.from().to_string(), None);
    let mut n = 100;
    let mut next = || {
        n += 1;
        fx.request(n)
    };

    let mut other_node = next();
    other_node.plan.message.as_mut().unwrap().sender_node = Some(NodeUuid::from_u128(9));
    fx.assert_refused(&other_node, UNVERIFIED, 55);

    let mut other_author = next();
    other_author.plan.message.as_mut().unwrap().author = Some(SeatId::new("sBBBBBBBBBBB"));
    fx.assert_refused(&other_author, UNVERIFIED, 55);

    let mut other_request = next();
    other_request.plan.message.as_mut().unwrap().request = Uuid::from_u128(7);
    fx.assert_refused(&other_request, UNVERIFIED, 55);

    let mut tampered = next();
    tampered.plan.message.as_mut().unwrap().body.push('!');
    fx.assert_refused(&tampered, UNVERIFIED, 55);

    // The signature is checked before the profile directory.
    let mut both = next();
    both.plan.message.as_mut().unwrap().body.push('!');
    both.plan.profile_dir = Some("relative".into());
    fx.assert_refused(&both, UNVERIFIED, 55);

    let dir = "profile directory is not an existing absolute directory here";
    let mut relative = next();
    relative.plan.profile_dir = Some("relative/dir".into());
    fx.assert_refused(&relative, dir, 60);
    let mut missing = next();
    missing.plan.profile_dir = Some(format!("{}/absent", fx.profile.display()));
    fx.assert_refused(&missing, dir, 60);
    let file = fx.profile.join("a-file");
    std::fs::write(&file, b"x").unwrap();
    let mut not_dir = next();
    not_dir.plan.profile_dir = Some(file.to_str().unwrap().into());
    fx.assert_refused(&not_dir, dir, 60);
}

// Kills: a pane plan carrying a profile directory accepted (Herdr's agent.start carries argv only,
// so the agent would run on the shell's default root while the CLI reports started), the refusal
// keyed on the handoff block instead of target and field, a refusal that is not recorded or
// writes a fact, and a changed 49-byte text.
#[test]
fn a_pane_plan_with_a_profile_dir_is_refused() {
    let fx = Fx::new("handoff-decide-pane-profile");
    fx.replicate_node();
    fx.replicate_seat(&fx.from().to_string(), None);
    let pane = LaunchTarget::Pane {
        space: None,
        tab: None,
        pane: "w1:p1".into(),
    };
    let detail = "a pane launch cannot change the profile directory";
    let mut handoff = fx.request(1);
    handoff.plan.target = pane.clone();
    fx.assert_refused(&handoff, detail, 49);
    // A repeat answers the recorded refusal.
    let entries = fx.journal_entries();
    assert_eq!(
        fx.launcher.launch(fx.from(), handoff.clone()),
        LaunchState::Refused {
            detail: detail.into()
        }
    );
    assert_eq!(fx.journal_entries(), entries);
    // A bare (non-handoff) pane plan is refused the same way.
    let bare = LaunchRequest {
        id: Uuid::from_u128(2),
        requester_seat: SEAT.into(),
        plan: LaunchPlan {
            target: pane,
            kind: fx.kind.into(),
            profile_dir: Some(fx.profile.to_str().unwrap().into()),
            ..LaunchPlan::default()
        },
    };
    fx.assert_refused(&bare, detail, 49);
}

// Kills: a sender seat homed on another node (or a local one) accepted: the 44-byte refusal
// precedes the profile directory check.
#[test]
fn a_seat_with_another_home_is_refused() {
    let fx = Fx::new("handoff-decide-foreign-seat");
    fx.replicate_node();
    fx.replicate_seat(&NodeUuid::from_u128(5).to_string(), None);
    let mut request = fx.request(1);
    request.plan.profile_dir = Some("relative".into());
    fx.assert_refused(&request, "the sender seat is not that node's seat here", 44);
}

// Kills: a retired author accepted, so frozen request bytes re-sent after the sender's seat
// retired still stage a task; and the check placed after the profile directory check (the
// 31-byte refusal precedes the 60-byte one).
#[test]
fn a_retired_sender_seat_is_refused() {
    let fx = Fx::new("handoff-decide-retired-seat");
    fx.replicate_node();
    fx.replicate_seat(&fx.from().to_string(), None);
    fx.rig
        .db()
        .execute(
            "UPDATE seats SET state='retired', retired_at=1, retired_seq=1 WHERE id=?1",
            params![SEAT],
        )
        .unwrap();
    let mut request = fx.request(1);
    request.plan.profile_dir = Some("relative".into());
    fx.assert_refused(&request, "the sender seat is retired here", 31);
}

// Kills: an unreplicated sender recorded as a refusal (a retry answered the stale refusal for
// good), and a retry that does not succeed once the node and seat replicate.
#[test]
fn an_unreplicated_sender_is_unavailable_unrecorded_and_a_retry_succeeds() {
    let fx = Fx::new("handoff-decide-unavailable");
    let request = fx.request(1);
    let entries = fx.journal_entries();
    let unavailable = |reason: &str, len: usize| {
        assert_eq!(reason.len(), len, "the stated byte size");
        LaunchState::Unavailable {
            reason: reason.into(),
        }
    };
    // Not even the node's key: checked before the signature, so no refusal.
    assert_eq!(
        fx.launcher.launch(fx.from(), request.clone()),
        unavailable("sender node key not replicated here yet", 39)
    );
    assert!(fx.row(&request).is_none());
    fx.replicate_node();
    assert_eq!(
        fx.launcher.launch(fx.from(), request.clone()),
        unavailable("sender seat not replicated here yet", 35)
    );
    assert!(fx.row(&request).is_none());
    assert_eq!(fx.journal_entries(), entries);
    // Once the seat replicates, the same id passes every handoff check.
    fx.replicate_seat(&fx.from().to_string(), None);
    let decision = fx
        .launcher
        .handoff_refusal(fx.from(), &request)
        .expect("accepted")
        .expect("a handoff");
    assert!(decision.prompt.starts_with(SUMMARY));
    assert!(fx.row(&request).is_none(), "the checks write nothing");
}

// Kills: a prompt that is not the one renderer's output, one that drops the seat's replicated
// name (or invents one), or one rendered from anything but the stored plan.
#[test]
fn valid_plan_freezes_the_rendered_prompt() {
    let named = Fx::new("handoff-decide-prompt-named");
    named.replicate_node();
    named.replicate_seat(&named.from().to_string(), Some("reviewer"));
    let request = named.request(1);
    assert_eq!(
        named
            .launcher
            .handoff_refusal(named.from(), &request)
            .unwrap()
            .unwrap()
            .prompt,
        render_v2(
            Some(SUMMARY),
            &SenderLine {
                name: Some("reviewer".into()),
                id: SEAT.into()
            }
        )
    );
    let anonymous = Fx::new("handoff-decide-prompt-anonymous");
    anonymous.replicate_node();
    anonymous.replicate_seat(&anonymous.from().to_string(), None);
    let prompt = anonymous
        .launcher
        .handoff_refusal(anonymous.from(), &anonymous.request(1))
        .unwrap()
        .unwrap()
        .prompt;
    assert_eq!(prompt, format!("{SUMMARY} \u{2014} from: {SEAT}"));
}

// Kills: a repeated id re-decided, re-validated or re-launched: in each recorded state the
// answer is the row's (`staged` included), the recorded row is untouched and no slot is taken.
#[test]
fn a_repeated_id_answers_the_recorded_outcome_and_starts_nothing() {
    let fx = Fx::new("handoff-decide-duplicate");
    // Nothing replicated: a fresh decision would answer `Unavailable`, so any other answer
    // came from the row.
    let from = fx.from().to_string();
    let seeded = |n: u128, state: &str, detail: Option<&str>, staged: bool| {
        let request = fx.request(n);
        let db = fx.rig.db();
        db.execute(
            "INSERT INTO peer_requests(id,node_uuid,requester_seat,kind,plan,state,pane,seat,detail,created_at,updated_at,handoff_thread,handoff_message) \
             VALUES (?1,?2,?3,'new_tab','{}',?4,'w1:p1',?5,?6,0,0,?7,?8)",
            params![
                request.id.to_string(),
                from,
                SEAT,
                state,
                (state == "launched").then_some("sCCCCCCCCCCC"),
                detail,
                staged.then_some("tTHREAD"),
                staged.then_some("mMESSAGE"),
            ],
        )
        .unwrap();
        request
    };
    let live_request = seeded(1, "running", None, false);
    fx.launcher
        .live
        .lock()
        .unwrap()
        .insert(live_request.id.to_string(), from.clone());
    for request in [
        live_request,
        seeded(2, "launched", None, false),
        seeded(
            3,
            "refused",
            Some("that pane is held for recovery here"),
            false,
        ),
        seeded(4, "failed", Some("child exited 1"), true),
        seeded(5, "failed", Some("child exited 1"), false),
    ] {
        let before = fx.row(&request).unwrap();
        let answer = fx.launcher.launch(fx.from(), request.clone());
        assert_eq!(answer, before.answer(), "state {}", before.state);
        assert_eq!(fx.row(&request).unwrap(), before, "the row is untouched");
    }
    assert!(matches!(
        fx.launcher.launch(fx.from(), fx.request(4)),
        LaunchState::Failed {
            staged: Some(_),
            ..
        }
    ));
    assert!(matches!(
        fx.launcher.launch(fx.from(), fx.request(5)),
        LaunchState::Failed { staged: None, .. }
    ));
    assert_eq!(fx.launcher.live.lock().unwrap().len(), 1, "no slot taken");
}

// Kills: an insert that drops `handoff_prompt` (the worker would launch with no first
// message), and one that keeps a prompt over a first writer's (a concurrent decision's row wins).
#[test]
fn the_row_insert_stores_the_frozen_prompt_once() {
    let fx = Fx::new("handoff-decide-insert");
    let request = fx.request(1);
    let new = |prompt: Option<&str>| peer_requests::PeerRequest {
        id: request.id.to_string(),
        node_uuid: fx.from().to_string(),
        requester_seat: Some(SEAT.into()),
        kind: "new_tab".into(),
        plan: "{}".into(),
        state: "running".into(),
        pane: None,
        seat: None,
        detail: None,
        handoff_thread: None,
        handoff_message: None,
        handoff_prompt: prompt.map(str::to_owned),
        created_at: 1,
        updated_at: 0,
    };
    let db = fx.rig.db();
    let (stored, inserted) = peer_requests::insert_or_get(&db, &new(Some("first"))).unwrap();
    assert!(inserted);
    assert_eq!(stored.handoff_prompt.as_deref(), Some("first"));
    let (stored, inserted) = peer_requests::insert_or_get(&db, &new(Some("second"))).unwrap();
    assert!(!inserted);
    assert_eq!(stored.handoff_prompt.as_deref(), Some("first"));
}
