//! Spec D7: the domain service stamps `mod_channel_live` on the attention
//! digest results from the shared mod-channel registry.

use crate::{
    ports::{LocalService, ModChannels, StorePort},
    protocol::{
        authority::PeerIdentity,
        commands::{AttentionDigestQuery, Command},
        ids::SeatId,
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
    let path = std::env::temp_dir().join(format!("mod-digest-flag-{}", uuid::Uuid::new_v4()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap();
    let clock: Arc<dyn Clock> = Arc::new(FixedClock);
    let context = StoreContext::new(path.join("store.db"), clock.clone());
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES ('i',0,'b',1)",
        [],
    )
    .unwrap();
    for seat in ["s", "other"] {
        db.execute(
            "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,'i','resolved','native',?1||'-target',0,0,0)",
            [seat],
        )
        .unwrap();
    }
    drop(db);
    let store: Arc<dyn StorePort> =
        Arc::new(SqliteStore::new(context, "i", StoreSettings::default()).unwrap());
    (
        DomainService::new("i".into(), store, clock).with_mod_channels(channels),
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
fn domain_sets_mod_channel_live_in_either_grace_state() {
    for state in [
        ModChannelState::RebindGrace,
        ModChannelState::ReconnectGrace,
    ] {
        let (service, _dir) = service(Arc::new(Fake(vec![entry("s", state)])));
        assert_eq!(flags(&service, "s"), (true, true), "{state:?}");
    }
}

#[test]
fn domain_leaves_mod_channel_live_false_without_a_channel() {
    let (inert, _dir) = service(Arc::new(crate::ports::NoModChannels));
    assert_eq!(flags(&inert, "s"), (false, false));
    let (empty, _dir2) = service(Arc::new(Fake(vec![])));
    assert_eq!(flags(&empty, "s"), (false, false));
}
