//! Spec D7: the domain service stamps `mod_channel_live` on the attention
//! digest results from the shared mod-channel registry.

use crate::{
    ports::{LocalService, ModChannels, StorePort},
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, PeerIdentity},
        commands::{AttentionDigestQuery, Command},
        ids::{BindingId, ExecutionId, HostTargetId, NativeSessionId, SeatId},
        results::CommandResult,
        time::{CallBudget, Clock, MonoInstant, UtcMillis},
        watch::{ModChannelEntry, ModChannelState, ModChannelStatus, ModDeliverySetting},
    },
    service::dispatch::DomainService,
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use std::{os::unix::fs::DirBuilderExt, sync::Arc};

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}

struct Fake(Vec<ModChannelEntry>);
/// A registry whose channels are keyed by binding: `binding_live` answers for the listed
/// `(seat, binding)` pairs only, `seat_live` for any of them.
struct ByBinding(Vec<(&'static str, &'static str)>);
impl ModChannels for ByBinding {
    fn status(&self) -> Option<ModChannelStatus> {
        None
    }
    fn binding_live(&self, seat: &SeatId, binding: &BindingId) -> bool {
        self.0
            .iter()
            .any(|(s, b)| *s == seat.as_str() && *b == binding.as_str())
    }
    fn seat_live(&self, seat: &SeatId) -> bool {
        self.0.iter().any(|(s, _)| *s == seat.as_str())
    }
}
impl ModChannels for Fake {
    fn status(&self) -> Option<ModChannelStatus> {
        Some(ModChannelStatus {
            mod_delivery: ModDeliverySetting::On,
            live_channels: self.0.len() as u32,
            channels: self.0.clone(),
        })
    }
}

fn entry(seat: &str, state: ModChannelState) -> ModChannelEntry {
    ModChannelEntry {
        seat: SeatId::new(seat),
        harness: "claude".into(),
        binding_generation: 3,
        connected_since: UtcMillis(1),
        state,
    }
}

struct Directory(std::path::PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn service(channels: Arc<dyn ModChannels>) -> (DomainService, Directory) {
    let path = crate::test_support::scratch::temp_path(format!(
        "mod-digest-flag-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap();
    let clock: Arc<dyn Clock> = Arc::new(FixedClock);
    let context = StoreContext::new(path.join("store.db"), clock.clone()).with_test_host();
    let db = context.open_writer().unwrap();
    db.execute("UPDATE herdr_hosts SET host_boot='b',host_epoch=1", [])
        .unwrap();
    for seat in ["s", "other"] {
        db.execute(
            "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES (?1,'34750f98bd59fcfc946da45aaabe933b','resolved','native',?1||'-target',0,0,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))",
            [seat],
        )
        .unwrap();
    }
    db.execute(
        "INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ((SELECT min(host_id) FROM herdr_hosts),'s-target','b',1,0,0,'fresh','term','inc','coherent_enumeration',1)",
        [],
    )
    .unwrap();
    // Two open Claude bindings of seat `s`: sessions `sess-a` and `sess-b`.
    for (binding, session) in [("bind-a", "sess-a"), ("bind-b", "sess-b")] {
        db.execute(
            "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation,binding_id) VALUES ('s',0,0,'s-target','b',1,'claude',?1,?2,'cooperative_top_level',0,'term','inc',?3)",
            rusqlite::params![session, uuid::Uuid::new_v4().to_string(), binding],
        )
        .unwrap();
    }
    drop(db);
    let store: Arc<dyn StorePort> = Arc::new(
        SqliteStore::new(
            context,
            "34750f98bd59fcfc946da45aaabe933b",
            StoreSettings::default(),
        )
        .unwrap(),
    );
    (
        DomainService::new("34750f98bd59fcfc946da45aaabe933b".into(), store, clock)
            .with_mod_channels(channels),
        Directory(path),
    )
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    }
}

/// `(AttentionDigest flag, AttentionDigestDelivery flag)` for a seat.
fn flags(service: &DomainService, seat: &str) -> (bool, bool) {
    let query = || AttentionDigestQuery {
        seat: SeatId::new(seat),
        lazy: false,
        claim: None,
        cli_hint: false,
    };
    let peer = || PeerIdentity::from_kernel(501);
    let plain = match service
        .handle(Command::AttentionDigest(query()), peer(), &budget())
        .unwrap()
    {
        CommandResult::AttentionDigest(digest) => digest.mod_channel_live,
        other => panic!("unexpected {other:?}"),
    };
    let delivery = match service
        .handle(Command::AttentionDigestDelivery(query()), peer(), &budget())
        .unwrap()
    {
        CommandResult::AttentionDigestDelivery { digest, .. } => digest.mod_channel_live,
        other => panic!("unexpected {other:?}"),
    };
    (plain, delivery)
}

#[test]
fn domain_sets_mod_channel_live_from_registry() {
    let live = Arc::new(Fake(vec![entry("s", ModChannelState::Live)]));
    let (service, _dir) = service(live);
    assert_eq!(flags(&service, "s"), (true, true));
    // Another seat's digest is not affected by this seat's channel.
    assert_eq!(flags(&service, "other"), (false, false));
}

#[test]
fn domain_sets_mod_channel_live_in_reconnect_grace_but_not_rebind_grace() {
    let (reconnect, _dir) = service(Arc::new(Fake(vec![entry(
        "s",
        ModChannelState::ReconnectGrace,
    )])));
    assert_eq!(flags(&reconnect, "s"), (true, true));
    // A rebind grace entry is a superseded binding: the new binding gets its digest.
    let (rebind, _dir2) = service(Arc::new(Fake(vec![entry(
        "s",
        ModChannelState::RebindGrace,
    )])));
    assert_eq!(flags(&rebind, "s"), (false, false));
}

#[test]
fn domain_leaves_mod_channel_live_false_without_a_channel() {
    let (inert, _dir) = service(Arc::new(crate::ports::NoModChannels));
    assert_eq!(flags(&inert, "s"), (false, false));
    let (empty, _dir2) = service(Arc::new(Fake(vec![])));
    assert_eq!(flags(&empty, "s"), (false, false));
}

/// A non-human claim for `session` of seat `s` (the SessionStart / hook caller).
fn claim(session: &str) -> CallerClaim {
    CallerClaim {
        host: crate::test_support::test_host_locator(),
        instance: "34750f98bd59fcfc946da45aaabe933b".into(),
        seat: SeatId::new("s"),
        binding_generation: 0,
        role: CallerRole::TopLevel,
        harness: Harness::Claude,
        native_session: NativeSessionId::new(session),
        execution: ExecutionId::new("execution"),
        target: HostTargetId::new("s-target"),
        agent_id: None,
        liveness: None,
    }
}

fn claimed_flags(service: &DomainService, claim: CallerClaim) -> (bool, bool) {
    let query = || AttentionDigestQuery {
        seat: SeatId::new("s"),
        lazy: false,
        claim: Some(claim.clone()),
        cli_hint: false,
    };
    let peer = || PeerIdentity::from_kernel(501);
    let plain = match service
        .handle(Command::AttentionDigest(query()), peer(), &budget())
        .unwrap()
    {
        CommandResult::AttentionDigest(digest) => digest.mod_channel_live,
        other => panic!("unexpected {other:?}"),
    };
    let delivery = match service
        .handle(Command::AttentionDigestDelivery(query()), peer(), &budget())
        .unwrap()
    {
        CommandResult::AttentionDigestDelivery { digest, .. } => digest.mod_channel_live,
        other => panic!("unexpected {other:?}"),
    };
    (plain, delivery)
}

/// Spec §2 Hook notifications / §3 Wake (ht-54r.55): the flag follows the binding the digest was
/// read for. A claim naming the binding whose channel is live gets `true`; a claim naming another
/// binding of the same seat gets `false` while that channel is live.
#[test]
fn domain_sets_mod_channel_live_for_the_claimed_binding_only() {
    let (service, _dir) = service(Arc::new(ByBinding(vec![("s", "bind-a")])));
    assert_eq!(claimed_flags(&service, claim("sess-a")), (true, true));
    assert_eq!(claimed_flags(&service, claim("sess-b")), (false, false));
    // No claim: the seat-level digest still follows the seat.
    assert_eq!(flags(&service, "s"), (true, true));
}

/// A non-human claim whose session has no binding yet has no channel, whatever else is live on
/// the seat (the `/clear` SessionStart reads the digest before its check-in enrolls the binding).
#[test]
fn domain_leaves_mod_channel_live_false_for_a_claim_without_a_binding() {
    let (service, _dir) = service(Arc::new(ByBinding(vec![("s", "bind-a")])));
    assert_eq!(claimed_flags(&service, claim("sess-new")), (false, false));
    // Same for an unenrolled subagent reading its parent's frontier.
    let mut subagent = claim("sess-a");
    subagent.role = CallerRole::Subagent;
    subagent.agent_id = Some(crate::protocol::ids::AgentId::new("agent-1"));
    assert_eq!(claimed_flags(&service, subagent), (false, false));
}
