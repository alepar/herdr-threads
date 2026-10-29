//! The sync coordinator's narrow store port.
//!
//! Responsibility (SyncStorePort / MemorySyncStore)
//! - Owns: the runtime's reads and writes of its own two tables:
//!   [`SyncStorePort::enabled_targets`], [`SyncStorePort::record_transition`],
//!   [`SyncStorePort::record_contacts`] and
//!   [`SyncStorePort::record_peer_status`] (and the link history,
//!   [`SyncStorePort::record_link_events`]). Each write takes its own
//!   `FairWriter` background turn, because the writes are rare and small.
//! - Hides: SQLite, the background turn and the store modules behind it.
//! - Must not: grow into a general store API; `StorePort` and `JournalPort`
//!   stay unextended. Operator writes (`add_explicit`, `forget`) and discovery
//!   (`upsert_discovered`) go straight to `store::sync_dial_targets`, not
//!   through this port.
//! - Role: implemented by `SqliteStore` (delegating to `store::sync_dial_targets`
//!   and `store::sync_contacts`) and by [`MemorySyncStore`] for coordinator and
//!   dialer tests. New runtime access to a sync table is a method here; the
//!   coordinator never touches `SqliteStore`, `StorePort` or SQL directly.
//!
//! Errors are store errors only: the Dialer logs a failed `record_transition`
//! and keeps its in-memory state, and a failed contact flush retries at the
//! next flush.

use crate::protocol::results::ApiError;
use crate::protocol::time::{CallBudget, UtcMillis};
use crate::store::remote_seat_status::RemoteSeatUpdate;
use crate::store::sync_contacts::NodeContact;
use crate::store::sync_dial_targets::{DialTarget, DialTransition, TargetId};
use crate::store::sync_link_events::LinkEvent;
use crate::store::sync_peer_status::PeerStatus;

/// See the module documentation.
pub trait SyncStorePort: Send + Sync {
    /// The enabled dial targets, by id.
    fn enabled_targets(&self, budget: &CallBudget) -> Result<Vec<DialTarget>, ApiError>;

    /// Stores what the Dialer observed for `target` at `now`. `InvalidRequest`
    /// when the target does not exist.
    fn record_transition(
        &self,
        target: TargetId,
        transition: &DialTransition,
        now: UtcMillis,
        budget: &CallBudget,
    ) -> Result<(), ApiError>;

    /// Upserts `contacts`, keeping the later `last_contact_at` per node.
    fn record_contacts(
        &self,
        contacts: &[NodeContact],
        budget: &CallBudget,
    ) -> Result<(), ApiError>;

    /// Upserts the peer nodes' last-known status (`store::sync_peer_status`).
    fn record_peer_status(&self, rows: &[PeerStatus], budget: &CallBudget) -> Result<(), ApiError>;

    /// Stores accepted remote seat-status snapshots
    /// (`store::remote_seat_status`).
    fn record_remote_seat_status(
        &self,
        updates: &[RemoteSeatUpdate],
        budget: &CallBudget,
    ) -> Result<(), ApiError>;

    /// Appends link up and down events to the `peer events` history
    /// (`store::sync_link_events`). Display data only; a store that keeps no
    /// history drops them.
    fn record_link_events(
        &self,
        events: &[LinkEvent],
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        let _ = (events, budget);
        Ok(())
    }
}

#[cfg(any(test, feature = "test-support"))]
pub use memory::MemorySyncStore;

#[cfg(any(test, feature = "test-support"))]
mod memory {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use super::SyncStorePort;
    use crate::node::NodeUuid;
    use crate::protocol::results::{ApiError, ErrorCode};
    use crate::protocol::time::{CallBudget, UtcMillis};
    use crate::store::remote_seat_status::RemoteSeatUpdate;
    use crate::store::sync_contacts::NodeContact;
    use crate::store::sync_dial_targets::{
        DialSource, DialState, DialTarget, DialTransition, TargetId,
    };
    use crate::store::sync_link_events::LinkEvent;
    use crate::store::sync_peer_status::PeerStatus;

    #[derive(Default)]
    struct State {
        targets: Vec<DialTarget>,
        contacts: BTreeMap<NodeUuid, NodeContact>,
        peers: BTreeMap<NodeUuid, PeerStatus>,
        seat_updates: Vec<RemoteSeatUpdate>,
        link_events: Vec<LinkEvent>,
    }

    /// An in-memory [`SyncStorePort`] for coordinator and dialer tests. It
    /// applies transitions with the same [`DialTarget::apply`] the SQLite
    /// store uses, and keeps the later contact per node.
    #[derive(Default)]
    pub struct MemorySyncStore {
        state: Mutex<State>,
    }

    impl MemorySyncStore {
        pub fn new() -> Self {
            Self::default()
        }

        /// Adds an enabled, idle explicit target and returns its id.
        pub fn add_target(&self, dest: &str) -> TargetId {
            let mut state = self.state.lock().unwrap();
            let id = TargetId(state.targets.len() as i64 + 1);
            state.targets.push(DialTarget {
                target_id: id,
                ssh_dest: dest.to_owned(),
                source: DialSource::Explicit,
                enabled: true,
                remote_command: None,
                learned_node: None,
                state: DialState::Idle,
                stopped: None,
                attempts: 0,
                next_attempt_at: None,
                last_error: None,
                last_error_at: None,
                last_linked_at: None,
                created_at: 0,
                updated_at: 0,
            });
            id
        }

        /// Gives the row `from` the destination `to`, as SQLite reusing a
        /// deleted row's id for a new destination does.
        pub fn rename(&self, from: &str, to: &str) {
            let mut state = self.state.lock().unwrap();
            for target in state.targets.iter_mut().filter(|t| t.ssh_dest == from) {
                to.clone_into(&mut target.ssh_dest);
            }
        }

        /// Disables the target with this destination.
        pub fn disable(&self, dest: &str) {
            let mut state = self.state.lock().unwrap();
            for target in state.targets.iter_mut().filter(|t| t.ssh_dest == dest) {
                target.enabled = false;
            }
        }

        /// What `add_explicit` does to an existing row: back to `idle` with no
        /// attempts, stop or retry time (the operator's "changed").
        pub fn reset(&self, dest: &str) {
            let mut state = self.state.lock().unwrap();
            for target in state.targets.iter_mut().filter(|t| t.ssh_dest == dest) {
                target.enabled = true;
                target.state = DialState::Idle;
                target.stopped = None;
                target.attempts = 0;
                target.next_attempt_at = None;
                target.updated_at = i64::MAX;
            }
        }

        /// Every target, enabled or not, by id.
        pub fn targets(&self) -> Vec<DialTarget> {
            self.state.lock().unwrap().targets.clone()
        }

        /// Every recorded contact, by node.
        pub fn contacts(&self) -> Vec<NodeContact> {
            self.state
                .lock()
                .unwrap()
                .contacts
                .values()
                .copied()
                .collect()
        }

        /// Every recorded peer status, by node (the latest row per node).
        pub fn peer_status(&self) -> Vec<PeerStatus> {
            self.state.lock().unwrap().peers.values().cloned().collect()
        }

        /// Every recorded link event, in order.
        pub fn link_events(&self) -> Vec<LinkEvent> {
            self.state.lock().unwrap().link_events.clone()
        }

        /// Every recorded seat-status update, in order.
        pub fn seat_updates(&self) -> Vec<RemoteSeatUpdate> {
            self.state.lock().unwrap().seat_updates.clone()
        }
    }

    impl SyncStorePort for MemorySyncStore {
        fn enabled_targets(&self, _: &CallBudget) -> Result<Vec<DialTarget>, ApiError> {
            Ok(self
                .state
                .lock()
                .unwrap()
                .targets
                .iter()
                .filter(|target| target.enabled)
                .cloned()
                .collect())
        }

        fn record_transition(
            &self,
            target: TargetId,
            transition: &DialTransition,
            now: UtcMillis,
            _: &CallBudget,
        ) -> Result<(), ApiError> {
            let mut state = self.state.lock().unwrap();
            let row = state
                .targets
                .iter_mut()
                .find(|row| row.target_id == target)
                .ok_or_else(|| ApiError::new(ErrorCode::InvalidRequest, "unknown dial target"))?;
            row.apply(transition, now.0);
            Ok(())
        }

        fn record_contacts(
            &self,
            contacts: &[NodeContact],
            _: &CallBudget,
        ) -> Result<(), ApiError> {
            let mut state = self.state.lock().unwrap();
            for contact in contacts {
                match state.contacts.get_mut(&contact.node) {
                    Some(held) if held.last_contact_at >= contact.last_contact_at => {}
                    Some(held) => *held = *contact,
                    None => {
                        state.contacts.insert(contact.node, *contact);
                    }
                }
            }
            Ok(())
        }

        fn record_peer_status(&self, rows: &[PeerStatus], _: &CallBudget) -> Result<(), ApiError> {
            let mut state = self.state.lock().unwrap();
            for row in rows {
                state.peers.insert(row.node, row.clone());
            }
            Ok(())
        }

        fn record_remote_seat_status(
            &self,
            updates: &[RemoteSeatUpdate],
            _: &CallBudget,
        ) -> Result<(), ApiError> {
            self.state
                .lock()
                .unwrap()
                .seat_updates
                .extend_from_slice(updates);
            Ok(())
        }

        fn record_link_events(&self, events: &[LinkEvent], _: &CallBudget) -> Result<(), ApiError> {
            self.state
                .lock()
                .unwrap()
                .link_events
                .extend_from_slice(events);
            Ok(())
        }
    }
}
