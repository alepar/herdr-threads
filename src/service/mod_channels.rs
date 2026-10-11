//! The daemon's registry of live push-channel mod watch channels (spec D2, D7; spec §3 Wake).
//!
//! One entry per binding: every connected top-level binding of a seat may hold its own channel
//! (`ModChannelRegistration` is decided against A2 by `DomainService::watch_register` before it
//! gets here), and a close, a grace, a stall or a cooldown applies to that binding only. The
//! registry is process-local and never durable. The channel's provenance
//! (`cooperative_mod_channel`) is audit only: it appears in one daemon-log line at registration
//! and grants no authority.
//!
//! # States
//!
//! - `Live`: a connection is attached; Attention frames are pushed to it.
//! - `ReconnectGrace`: the connection dropped without a Close; for
//!   [`MOD_RECONNECT_GRACE_MS`] the entry still counts as live (so wake
//!   pokes do not fire while the mod restarts its watch). Expiry removes the
//!   entry and kicks the wake lane.
//! - `RebindGrace`: after `Close{binding_changed}`, the binding's entry keeps authorizing that
//!   binding's late mod acks ([`ModChannels::is_live`]) for [`MOD_REBIND_GRACE_MS`]; it no
//!   longer suppresses wakes or the SessionStart digest (`seat_live` and `binding_live` skip
//!   it: the superseding binding is a new binding that must be woken for its own unread
//!   mail). A registration for the binding ends it; expiry removes the entry and kicks.
//!
//! `retired`, `unresolved`, `stalled`, `disabled` and `stopping` closes remove
//! the entry at once and kick the wake lane; `stalled` also starts the
//! 10-minute cooldown for that binding.
//!
//! # Ownership
//!
//! This module is the sole owner of the disconnect kick (the wake lane is
//! kicked whenever an entry stops being live) and of the stall state: the
//! stall predicate (spec D7) is `last_ack_or_registration` older than
//! [`MOD_STALL_AFTER_MS`], an Attention push on record, and an ordinary,
//! non-truncated pending receipt published before that push and itself older
//! than [`MOD_STALL_AFTER_MS`].
//!
//! A binding that disconnects or ends loses its channel through [`ModChannelRegistry::pass`]:
//! the store's disconnect hook (`mod_channel_release`) bumps the seat's attention version, the
//! observer marks the registry dirty, and the pass judges each entry against its own binding
//! (`BindingChanged`, that binding only).
//!
//! The worker (`start_mod_channel_worker`) calls [`ModChannelRegistry::pass`]
//! when a commit touched an attention source and [`ModChannelRegistry::sweep`]
//! every second. The registry never holds its lock across a store read.

use crate::{
    harness::adapter::IdleWake,
    ports::{
        ModBindingRef, ModChannelId, ModChannelRegistration, ModChannelSink, ModChannels,
        ModFingerprint, ModStoreReads,
    },
    protocol::{
        authority::Harness,
        ids::{BindingId, SeatId},
        results::ApiError,
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
        watch::{
            MAX_WATCH_CONNECTIONS, MOD_REBIND_GRACE_MS, MOD_RECONNECT_GRACE_MS, MOD_STALL_AFTER_MS,
            MOD_STALL_COOLDOWN_MS, ModChannelEntry, ModChannelState, ModChannelStatus,
            ModDeliverySetting, WATCH_BODY_LIMIT_BYTES, WatchCloseReason, WatchFrame,
            WatchRefusalReason,
        },
    },
};
use std::{
    collections::{BTreeSet, HashMap},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};

/// Budget of one store read made for a stall decision.
const STALL_READ_BUDGET_MS: u64 = 1_000;

struct Entry {
    seat: SeatId,
    channel: Option<ModChannelId>,
    sink: Option<Arc<dyn ModChannelSink>>,
    generation: u64,
    native_session: String,
    harness: Harness,
    idle_wake: IdleWake,
    connected_since: UtcMillis,
    state: ModChannelState,
    grace_until: Option<UtcMillis>,
    last_ack_or_registration: UtcMillis,
    last_attention_push: Option<UtcMillis>,
    last_fingerprint: Option<ModFingerprint>,
}

impl Entry {
    /// The entry counts as handled: its grace (if any) has not expired.
    fn unexpired(&self, now: UtcMillis) -> bool {
        self.grace_until.is_none_or(|until| until > now)
    }

    /// The entry means the mod is handling its binding: unexpired and not in `RebindGrace`
    /// (a closed binding is disconnected and no wake target; its grace only authorizes acks).
    fn handling(&self, now: UtcMillis) -> bool {
        self.unexpired(now) && self.state != ModChannelState::RebindGrace
    }
}

/// What an unlocked judgement saw: a close that follows a store read only
/// acts on the very entry it judged.
#[derive(Clone, Copy)]
struct Judged {
    channel: ModChannelId,
    /// `last_ack_or_registration` the stall judgement saw, when it was one.
    ack: Option<UtcMillis>,
}

struct State {
    entries: HashMap<BindingId, Entry>,
    /// The seat's bindings that hold an entry (kept in step with `entries`).
    by_seat: HashMap<SeatId, BTreeSet<BindingId>>,
    /// Binding -> refused until.
    cooldowns: HashMap<BindingId, UtcMillis>,
    mod_delivery: ModDeliverySetting,
    next_id: u64,
}

impl State {
    fn insert(&mut self, binding: BindingId, entry: Entry) {
        self.by_seat
            .entry(entry.seat.clone())
            .or_default()
            .insert(binding.clone());
        self.entries.insert(binding, entry);
    }

    fn remove(&mut self, binding: &BindingId) -> Option<Entry> {
        let entry = self.entries.remove(binding)?;
        if let Some(set) = self.by_seat.get_mut(&entry.seat) {
            set.remove(binding);
            if set.is_empty() {
                self.by_seat.remove(&entry.seat);
            }
        }
        Some(entry)
    }

    /// The seat's bindings that hold an entry.
    fn seat_bindings(&self, seat: &SeatId) -> Vec<BindingId> {
        self.by_seat
            .get(seat)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }
}

pub struct ModChannelRegistry {
    clock: Arc<dyn Clock>,
    store: Arc<dyn ModStoreReads>,
    instance: String,
    kick_wakes: Arc<dyn Fn() + Send + Sync>,
    log: Arc<dyn Fn(&str) + Send + Sync>,
    wake_worker: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    dirty: AtomicBool,
    state: Mutex<State>,
}

impl ModChannelRegistry {
    pub fn new(
        clock: Arc<dyn Clock>,
        store: Arc<dyn ModStoreReads>,
        instance: String,
        kick_wakes: Arc<dyn Fn() + Send + Sync>,
        mod_delivery: ModDeliverySetting,
    ) -> Self {
        Self {
            clock,
            store,
            instance,
            kick_wakes,
            log: Arc::new(|_| {}),
            wake_worker: Mutex::new(None),
            dirty: AtomicBool::new(false),
            state: Mutex::new(State {
                entries: HashMap::new(),
                by_seat: HashMap::new(),
                cooldowns: HashMap::new(),
                mod_delivery,
                next_id: 1,
            }),
        }
    }

    /// Where the audit line of a registration goes (the daemon log).
    #[must_use]
    pub fn with_log(mut self, log: Arc<dyn Fn(&str) + Send + Sync>) -> Self {
        self.log = log;
        self
    }

    pub fn instance(&self) -> &str {
        &self.instance
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The worker's wake hook: called when a registration needs its initial
    /// sweep and by [`Self::observer`].
    pub fn set_worker_wake(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        *self.wake_worker.lock().unwrap_or_else(|e| e.into_inner()) = Some(wake);
    }

    fn wake_worker(&self) {
        let wake = self
            .wake_worker
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(wake) = wake {
            wake();
        }
    }

    /// The observer a commit touching an attention source calls: marks the
    /// registry dirty and wakes the worker. Never does store work.
    pub fn observer(self: &Arc<Self>) -> Arc<dyn Fn() + Send + Sync> {
        let registry = Arc::downgrade(self);
        Arc::new(move || {
            if let Some(registry) = registry.upgrade() {
                registry.dirty.store(true, Ordering::SeqCst);
                registry.wake_worker();
            }
        })
    }

    /// Asks the worker for another pass (a pass failed).
    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::SeqCst);
    }

    /// True once per observed commit (or registration) since the last call.
    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::SeqCst)
    }

    fn kick(&self, times: usize) {
        for _ in 0..times {
            (self.kick_wakes)();
        }
    }

    fn call_budget(&self, millis: u64) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(self.clock.monotonic_now().0.saturating_add(millis)),
            cancellation: Cancellation::default(),
        }
    }

    /// The daemon setting. `Off` closes every channel with `Disabled`.
    pub fn set_mod_delivery(&self, setting: ModDeliverySetting) {
        self.lock().mod_delivery = setting;
        if setting == ModDeliverySetting::Off {
            self.close_all(WatchCloseReason::Disabled, self.clock.utc_now());
        }
    }

    /// Closes every channel with `reason` (daemon stop, setting off).
    pub fn close_all(&self, reason: WatchCloseReason, now: UtcMillis) {
        let bindings: Vec<BindingId> = self.lock().entries.keys().cloned().collect();
        for binding in bindings {
            self.close_inner(&binding, None, reason, now);
        }
    }

    /// Removes expired graces and cooldowns (kicking the wake lane for each
    /// removed entry) and closes channels whose stall predicate holds.
    pub fn sweep(&self, now: UtcMillis) {
        let removed = {
            let mut state = self.lock();
            let expired: Vec<BindingId> = state
                .entries
                .iter()
                .filter(|(_, entry)| entry.grace_until.is_some_and(|until| until <= now))
                .map(|(binding, _)| binding.clone())
                .collect();
            for binding in &expired {
                state.remove(binding);
            }
            state.cooldowns.retain(|_, until| *until > now);
            expired.len()
        };
        self.kick(removed);
        let candidates: Vec<BindingId> = self
            .lock()
            .entries
            .iter()
            .filter(|(_, entry)| {
                matches!(
                    entry.state,
                    ModChannelState::Live | ModChannelState::ReconnectGrace
                )
            })
            .map(|(binding, _)| binding.clone())
            .collect();
        for binding in candidates {
            if let Some((channel, judged_ack)) = self.stalled_judgement(&binding, now) {
                self.close_inner(
                    &binding,
                    Some(Judged {
                        channel,
                        ack: Some(judged_ack),
                    }),
                    WatchCloseReason::Stalled,
                    now,
                );
            }
        }
    }

    /// One worker pass: re-reads every binding with a Live or ReconnectGrace
    /// entry, closes channels whose binding or seat state no longer holds, and
    /// pushes Attention where the binding's fingerprint changed. A store error is
    /// returned after every entry was tried.
    pub fn pass(&self, now: UtcMillis) -> Result<(), ApiError> {
        let entries: Vec<(BindingId, SeatId, ModChannelId, String)> = self
            .lock()
            .entries
            .iter()
            .filter(|(_, entry)| {
                matches!(
                    entry.state,
                    ModChannelState::Live | ModChannelState::ReconnectGrace
                )
            })
            .filter_map(|(binding, entry)| {
                Some((
                    binding.clone(),
                    entry.seat.clone(),
                    entry.channel?,
                    entry.native_session.clone(),
                ))
            })
            .collect();
        let budget = self.call_budget(2_000);
        let mut first_error = None;
        for (binding, seat, channel, native_session) in entries {
            let judged = Some(Judged { channel, ack: None });
            let view =
                match self
                    .store
                    .mod_binding_view(&seat, ModBindingRef::Id(&binding), &budget)
                {
                    Ok(view) => view,
                    Err(error) => {
                        first_error.get_or_insert(error);
                        continue;
                    }
                };
            let Some(view) = view else {
                self.close_inner(&binding, judged, WatchCloseReason::Unresolved, now);
                continue;
            };
            if view.retired {
                self.close_inner(&binding, judged, WatchCloseReason::Retired, now);
                continue;
            }
            if !view.continuity_resolved {
                self.close_inner(&binding, judged, WatchCloseReason::Unresolved, now);
                continue;
            }
            let binding_holds = view.binding.as_ref().is_some_and(|held| {
                held.binding_id == binding
                    && held.holds_channel()
                    && held.native_session == native_session
            });
            if !binding_holds {
                self.close_inner(&binding, judged, WatchCloseReason::BindingChanged, now);
                continue;
            }
            self.observe_fingerprint(
                &binding,
                channel,
                view.attention_version,
                view.fingerprint,
                now,
            );
        }
        first_error.map_or(Ok(()), Err)
    }

    /// The body of [`ModChannels::close`]. With `judged`, it returns under the
    /// lock without acting when the entry is no longer the one judged (a
    /// registration landed meanwhile; `observe_fingerprint` does the same).
    fn close_inner(
        &self,
        binding: &BindingId,
        judged: Option<Judged>,
        reason: WatchCloseReason,
        now: UtcMillis,
    ) {
        let kicked = {
            let mut state = self.lock();
            let Some(entry) = state.entries.get_mut(binding) else {
                return;
            };
            if let Some(judged) = judged
                && (entry.channel != Some(judged.channel)
                    || judged
                        .ack
                        .is_some_and(|ack| entry.last_ack_or_registration != ack))
            {
                return;
            }
            if let Some(sink) = entry.sink.take() {
                sink.push(WatchFrame::Close { reason });
            }
            match reason {
                WatchCloseReason::BindingChanged => {
                    if entry.state != ModChannelState::RebindGrace {
                        entry.state = ModChannelState::RebindGrace;
                        entry.grace_until =
                            Some(UtcMillis(now.0.saturating_add(MOD_REBIND_GRACE_MS as i64)));
                        entry.channel = None;
                    }
                    false
                }
                // A newer channel took over: nothing to remove or kick.
                WatchCloseReason::Replaced => false,
                _ => {
                    state.remove(binding);
                    if reason == WatchCloseReason::Stalled {
                        state.cooldowns.insert(
                            binding.clone(),
                            UtcMillis(now.0.saturating_add(MOD_STALL_COOLDOWN_MS as i64)),
                        );
                    }
                    true
                }
            }
        };
        if kicked {
            self.kick(1);
        }
    }

    /// The stall predicate (spec D7) for one binding's channel. When it holds, the channel id
    /// and the `last_ack_or_registration` it judged, so the close can verify that the entry is
    /// unchanged.
    fn stalled_judgement(
        &self,
        binding: &BindingId,
        now: UtcMillis,
    ) -> Option<(ModChannelId, UtcMillis)> {
        let (seat, channel, judged_ack, pushed) = {
            let state = self.lock();
            let entry = state.entries.get(binding)?;
            if !matches!(
                entry.state,
                ModChannelState::Live | ModChannelState::ReconnectGrace
            ) {
                return None;
            }
            if now.0.saturating_sub(entry.last_ack_or_registration.0) <= MOD_STALL_AFTER_MS as i64 {
                return None;
            }
            (
                entry.seat.clone(),
                entry.channel?,
                entry.last_ack_or_registration,
                entry.last_attention_push?,
            )
        };
        let budget = self.call_budget(STALL_READ_BUDGET_MS);
        match self
            .store
            .mod_stall_oldest(&seat, pushed, WATCH_BODY_LIMIT_BYTES, &budget)
        {
            Ok(Some(published))
                if now.0.saturating_sub(published.0) > MOD_STALL_AFTER_MS as i64 =>
            {
                Some((channel, judged_ack))
            }
            _ => None,
        }
    }

    /// Pushes Attention to `binding`'s channel when `fingerprint` differs from the last one
    /// seen. The first reading of a fresh channel pushes only if something is pending (the
    /// registration's initial sweep). Does nothing when `channel` is no longer the binding's.
    fn observe_fingerprint(
        &self,
        binding: &BindingId,
        channel: ModChannelId,
        version: u64,
        fingerprint: ModFingerprint,
        now: UtcMillis,
    ) {
        let push = {
            let mut state = self.lock();
            let Some(entry) = state.entries.get_mut(binding) else {
                return;
            };
            if entry.channel != Some(channel) {
                return;
            }
            let changed = match &entry.last_fingerprint {
                None => fingerprint.has_pending(),
                Some(last) => *last != fingerprint,
            };
            entry.last_fingerprint = Some(fingerprint);
            changed && entry.state == ModChannelState::Live
        };
        if push {
            self.notify_binding(binding, version, now);
        }
    }

    fn notify_binding(&self, binding: &BindingId, version: u64, now: UtcMillis) {
        let mut state = self.lock();
        let Some(entry) = state.entries.get_mut(binding) else {
            return;
        };
        if entry.state != ModChannelState::Live {
            return;
        }
        let Some(sink) = entry.sink.clone() else {
            return;
        };
        if sink.push(WatchFrame::Attention { version }) {
            entry.last_attention_push = Some(now);
        }
    }
}

impl ModChannels for ModChannelRegistry {
    fn register(
        &self,
        registration: ModChannelRegistration,
        sink: Arc<dyn ModChannelSink>,
    ) -> Result<ModChannelId, WatchRefusalReason> {
        let now = registration.registered_at;
        let id = {
            let mut state = self.lock();
            if state.mod_delivery == ModDeliverySetting::Off {
                return Err(WatchRefusalReason::Disabled);
            }
            if state
                .cooldowns
                .get(&registration.binding)
                .is_some_and(|until| *until > now)
            {
                return Err(WatchRefusalReason::Cooldown);
            }
            if !state.entries.contains_key(&registration.binding)
                && state.entries.len() >= MAX_WATCH_CONNECTIONS
            {
                return Err(WatchRefusalReason::Busy);
            }
            if let Some(old) = state.entries.get(&registration.binding)
                && old.state == ModChannelState::Live
                && let Some(old_sink) = &old.sink
            {
                old_sink.push(WatchFrame::Close {
                    reason: WatchCloseReason::Replaced,
                });
            }
            let id = ModChannelId(state.next_id);
            state.next_id += 1;
            state.insert(
                registration.binding.clone(),
                Entry {
                    seat: registration.seat.clone(),
                    channel: Some(id),
                    sink: Some(sink),
                    generation: registration.binding_generation,
                    native_session: registration.native_session.as_str().to_owned(),
                    harness: registration.harness,
                    idle_wake: registration.idle_wake,
                    connected_since: now,
                    state: ModChannelState::Live,
                    grace_until: None,
                    last_ack_or_registration: now,
                    last_attention_push: None,
                    last_fingerprint: None,
                },
            );
            id
        };
        (self.log)(&format!(
            "mod channel registered seat={} binding={} generation={} provenance={}",
            registration.seat.as_str(),
            registration.binding.as_str(),
            registration.binding_generation,
            crate::protocol::authority::COOPERATIVE_MOD_CHANNEL_PROVENANCE
        ));
        self.dirty.store(true, Ordering::SeqCst);
        self.wake_worker();
        Ok(id)
    }

    fn unregister(&self, channel: ModChannelId, now: UtcMillis) {
        let mut state = self.lock();
        let Some(entry) = state
            .entries
            .values_mut()
            .find(|entry| entry.channel == Some(channel))
        else {
            return;
        };
        if entry.state != ModChannelState::Live {
            return;
        }
        entry.state = ModChannelState::ReconnectGrace;
        entry.grace_until = Some(UtcMillis(
            now.0.saturating_add(MOD_RECONNECT_GRACE_MS as i64),
        ));
        entry.sink = None;
    }

    fn close(&self, binding: &BindingId, reason: WatchCloseReason, now: UtcMillis) {
        self.close_inner(binding, None, reason, now);
    }

    fn is_live(&self, seat: &SeatId, binding: &BindingId) -> bool {
        let now = self.clock.utc_now();
        self.lock()
            .entries
            .get(binding)
            .is_some_and(|entry| &entry.seat == seat && entry.unexpired(now))
    }

    fn binding_live(&self, seat: &SeatId, binding: &BindingId) -> bool {
        let now = self.clock.utc_now();
        self.lock()
            .entries
            .get(binding)
            .is_some_and(|entry| &entry.seat == seat && entry.handling(now))
    }

    fn notify(&self, seat: &SeatId, attention_version: u64) {
        let now = self.clock.utc_now();
        let bindings = self.lock().seat_bindings(seat);
        for binding in bindings {
            self.notify_binding(&binding, attention_version, now);
        }
    }

    fn seat_live(&self, seat: &SeatId) -> bool {
        let now = self.clock.utc_now();
        let state = self.lock();
        state.by_seat.get(seat).is_some_and(|bindings| {
            bindings
                .iter()
                .any(|binding| state.entries.get(binding).is_some_and(|e| e.handling(now)))
        })
    }

    fn wake_suppressed(&self, seat: &SeatId) -> bool {
        // Per binding (spec §3 Wake): some handling channel of the seat (Live or
        // ReconnectGrace, as `seat_live`) whose policy wakes an idle session itself.
        let now = self.clock.utc_now();
        let state = self.lock();
        state.by_seat.get(seat).is_some_and(|bindings| {
            bindings.iter().any(|binding| {
                state
                    .entries
                    .get(binding)
                    .is_some_and(|e| e.idle_wake == IdleWake::Channel && e.handling(now))
            })
        })
    }

    fn record_attention_push(&self, seat: &SeatId, binding: &BindingId, now: UtcMillis) {
        if let Some(entry) = self.lock().entries.get_mut(binding)
            && &entry.seat == seat
            && matches!(
                entry.state,
                ModChannelState::Live | ModChannelState::ReconnectGrace
            )
        {
            entry.last_attention_push = Some(now);
        }
    }

    fn record_ack(&self, seat: &SeatId, binding: &BindingId, now: UtcMillis) {
        if let Some(entry) = self.lock().entries.get_mut(binding)
            && &entry.seat == seat
            && matches!(
                entry.state,
                ModChannelState::Live | ModChannelState::ReconnectGrace
            )
        {
            entry.last_ack_or_registration = now;
        }
    }

    fn binding_stalled(&self, seat: &SeatId, binding: &BindingId, now: UtcMillis) -> bool {
        let held = self
            .lock()
            .entries
            .get(binding)
            .is_some_and(|entry| &entry.seat == seat);
        held && self.stalled_judgement(binding, now).is_some()
    }

    fn stalled(&self, seat: &SeatId, now: UtcMillis) -> bool {
        let bindings: Vec<BindingId> = {
            let state = self.lock();
            state
                .seat_bindings(seat)
                .into_iter()
                .filter(|binding| state.entries.get(binding).is_some_and(|e| e.unexpired(now)))
                .collect()
        };
        !bindings.is_empty()
            && bindings
                .iter()
                .all(|binding| self.stalled_judgement(binding, now).is_some())
    }

    fn status(&self) -> Option<ModChannelStatus> {
        let state = self.lock();
        let mut channels: Vec<ModChannelEntry> = state
            .entries
            .values()
            .map(|entry| ModChannelEntry {
                seat: entry.seat.clone(),
                harness: entry.harness.as_str().to_owned(),
                binding_generation: entry.generation,
                connected_since: entry.connected_since,
                state: entry.state,
            })
            .collect();
        channels.sort_by(|a, b| {
            a.seat
                .cmp(&b.seat)
                .then(a.connected_since.0.cmp(&b.connected_since.0))
                .then(a.binding_generation.cmp(&b.binding_generation))
        });
        channels.truncate(MAX_WATCH_CONNECTIONS);
        Some(ModChannelStatus {
            mod_delivery: state.mod_delivery,
            live_channels: u32::try_from(
                state
                    .entries
                    .values()
                    .filter(|entry| entry.state == ModChannelState::Live)
                    .count(),
            )
            .unwrap_or(u32::MAX),
            channels,
        })
    }
}

#[cfg(test)]
#[path = "../../tests/service/mod_channels.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/delivery/push_channel.rs"]
mod push_channel_tests;
