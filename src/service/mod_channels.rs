//! The daemon's registry of live Claude mod watch channels (spec D2, D7).
//!
//! One entry per seat (`ModChannelRegistration` is decided against A2 by
//! `DomainService::watch_register` before it gets here); the registry is
//! process-local and never durable. The channel's provenance
//! (`cooperative_mod_channel`) is audit only: it appears in one daemon-log
//! line at registration and grants no authority.
//!
//! # States
//!
//! - `Live`: a connection is attached; Attention frames are pushed to it.
//! - `ReconnectGrace`: the connection dropped without a Close; for
//!   [`MOD_RECONNECT_GRACE_MS`] the entry still counts as live (so wake
//!   pokes do not fire while the mod restarts its watch). Expiry removes the
//!   entry and kicks the wake lane.
//! - `RebindGrace`: after `Close{binding_changed}`, the seat counts as live
//!   for every binding generation for [`MOD_REBIND_GRACE_MS`] (the SessionStart
//!   check-in that rotated the generation then omits its digest). A
//!   registration ends it; expiry removes the entry and kicks.
//!
//! `retired`, `unresolved`, `stalled`, `disabled` and `stopping` closes remove
//! the entry at once and kick the wake lane; `stalled` also starts the
//! 10-minute cooldown for that binding generation.
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
//! The worker (`start_mod_channel_worker`) calls [`ModChannelRegistry::pass`]
//! when a commit touched an attention source and [`ModChannelRegistry::sweep`]
//! every second. The registry never holds its lock across a store read.

use crate::{
    ports::{
        ModChannelId, ModChannelRegistration, ModChannelSink, ModChannels, ModFingerprint,
        ModStoreReads,
    },
    protocol::{
        authority::Harness,
        ids::SeatId,
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
    collections::HashMap,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};

/// Budget of one store read made for a stall decision.
const STALL_READ_BUDGET_MS: u64 = 1_000;

struct Entry {
    channel: Option<ModChannelId>,
    sink: Option<Arc<dyn ModChannelSink>>,
    generation: u64,
    native_session: String,
    harness: Harness,
    connected_since: UtcMillis,
    state: ModChannelState,
    grace_until: Option<UtcMillis>,
    last_ack_or_registration: UtcMillis,
    last_attention_push: Option<UtcMillis>,
    last_fingerprint: Option<ModFingerprint>,
}

struct State {
    entries: HashMap<SeatId, Entry>,
    /// `(seat, binding generation)` -> refused until.
    cooldowns: HashMap<(SeatId, u64), UtcMillis>,
    mod_delivery: ModDeliverySetting,
    next_id: u64,
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
        let seats: Vec<SeatId> = self.lock().entries.keys().cloned().collect();
        for seat in seats {
            self.close(&seat, reason, now);
        }
    }

    /// Removes expired graces and cooldowns (kicking the wake lane for each
    /// removed entry) and closes channels whose stall predicate holds.
    pub fn sweep(&self, now: UtcMillis) {
        let removed = {
            let mut state = self.lock();
            let expired: Vec<SeatId> = state
                .entries
                .iter()
                .filter(|(_, entry)| entry.grace_until.is_some_and(|until| until <= now))
                .map(|(seat, _)| seat.clone())
                .collect();
            for seat in &expired {
                state.entries.remove(seat);
            }
            state.cooldowns.retain(|_, until| *until > now);
            expired.len()
        };
        self.kick(removed);
        let candidates: Vec<SeatId> = self
            .lock()
            .entries
            .iter()
            .filter(|(_, entry)| {
                matches!(
                    entry.state,
                    ModChannelState::Live | ModChannelState::ReconnectGrace
                )
            })
            .map(|(seat, _)| seat.clone())
            .collect();
        for seat in candidates {
            if self.stalled(&seat, now) {
                self.close(&seat, WatchCloseReason::Stalled, now);
            }
        }
    }

    /// One worker pass: re-reads every seat with a Live or ReconnectGrace
    /// entry, closes channels whose binding or seat state no longer holds, and
    /// pushes Attention where the seat's fingerprint changed. A store error is
    /// returned after every seat was tried.
    pub fn pass(&self, now: UtcMillis) -> Result<(), ApiError> {
        let seats: Vec<(SeatId, u64, String)> = self
            .lock()
            .entries
            .iter()
            .filter(|(_, entry)| {
                matches!(
                    entry.state,
                    ModChannelState::Live | ModChannelState::ReconnectGrace
                )
            })
            .map(|(seat, entry)| (seat.clone(), entry.generation, entry.native_session.clone()))
            .collect();
        let budget = self.call_budget(2_000);
        let mut first_error = None;
        for (seat, generation, native_session) in seats {
            let view = match self.store.mod_seat_view(&seat, &budget) {
                Ok(view) => view,
                Err(error) => {
                    first_error.get_or_insert(error);
                    continue;
                }
            };
            let Some(view) = view else {
                self.close(&seat, WatchCloseReason::Unresolved, now);
                continue;
            };
            if view.retired {
                self.close(&seat, WatchCloseReason::Retired, now);
                continue;
            }
            if !view.continuity_resolved {
                self.close(&seat, WatchCloseReason::Unresolved, now);
                continue;
            }
            let binding_holds = view.binding.as_ref().is_some_and(|binding| {
                binding.generation == generation && binding.native_session == native_session
            });
            if !binding_holds {
                self.close(&seat, WatchCloseReason::BindingChanged, now);
                continue;
            }
            self.observe_fingerprint(
                &seat,
                generation,
                view.attention_version,
                view.fingerprint,
                now,
            );
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Pushes Attention when `fingerprint` differs from the last one seen.
    /// The first reading of a fresh channel pushes only if something is
    /// pending (the registration's initial sweep).
    fn observe_fingerprint(
        &self,
        seat: &SeatId,
        generation: u64,
        version: u64,
        fingerprint: ModFingerprint,
        now: UtcMillis,
    ) {
        let push = {
            let mut state = self.lock();
            let Some(entry) = state.entries.get_mut(seat) else {
                return;
            };
            if entry.generation != generation {
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
            self.notify_at(seat, version, now);
        }
    }

    fn notify_at(&self, seat: &SeatId, version: u64, now: UtcMillis) {
        let mut state = self.lock();
        let Some(entry) = state.entries.get_mut(seat) else {
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
                .get(&(registration.seat.clone(), registration.binding_generation))
                .is_some_and(|until| *until > now)
            {
                return Err(WatchRefusalReason::Cooldown);
            }
            if !state.entries.contains_key(&registration.seat)
                && state.entries.len() >= MAX_WATCH_CONNECTIONS
            {
                return Err(WatchRefusalReason::Busy);
            }
            if let Some(old) = state.entries.get(&registration.seat)
                && old.state == ModChannelState::Live
                && let Some(old_sink) = &old.sink
            {
                old_sink.push(WatchFrame::Close {
                    reason: WatchCloseReason::Replaced,
                });
            }
            let id = ModChannelId(state.next_id);
            state.next_id += 1;
            state.entries.insert(
                registration.seat.clone(),
                Entry {
                    channel: Some(id),
                    sink: Some(sink),
                    generation: registration.binding_generation,
                    native_session: registration.native_session.as_str().to_owned(),
                    harness: registration.harness,
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
            "mod channel registered seat={} generation={} provenance=cooperative_mod_channel",
            registration.seat.as_str(),
            registration.binding_generation
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

    fn close(&self, seat: &SeatId, reason: WatchCloseReason, now: UtcMillis) {
        let kicked = {
            let mut state = self.lock();
            let Some(entry) = state.entries.get_mut(seat) else {
                return;
            };
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
                    let generation = entry.generation;
                    state.entries.remove(seat);
                    if reason == WatchCloseReason::Stalled {
                        state.cooldowns.insert(
                            (seat.clone(), generation),
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

    fn is_live(&self, seat: &SeatId, binding_generation: u64) -> bool {
        let now = self.clock.utc_now();
        let state = self.lock();
        let Some(entry) = state.entries.get(seat) else {
            return false;
        };
        if entry.grace_until.is_some_and(|until| until <= now) {
            return false;
        }
        match entry.state {
            ModChannelState::Live | ModChannelState::ReconnectGrace => {
                entry.generation == binding_generation
            }
            ModChannelState::RebindGrace => true,
        }
    }

    fn notify(&self, seat: &SeatId, attention_version: u64) {
        self.notify_at(seat, attention_version, self.clock.utc_now());
    }

    fn record_attention_push(&self, seat: &SeatId, now: UtcMillis) {
        if let Some(entry) = self.lock().entries.get_mut(seat) {
            entry.last_attention_push = Some(now);
        }
    }

    fn record_ack(&self, seat: &SeatId, now: UtcMillis) {
        if let Some(entry) = self.lock().entries.get_mut(seat) {
            entry.last_ack_or_registration = now;
        }
    }

    fn stalled(&self, seat: &SeatId, now: UtcMillis) -> bool {
        let pushed = {
            let state = self.lock();
            let Some(entry) = state.entries.get(seat) else {
                return false;
            };
            if !matches!(
                entry.state,
                ModChannelState::Live | ModChannelState::ReconnectGrace
            ) {
                return false;
            }
            if now.0.saturating_sub(entry.last_ack_or_registration.0) <= MOD_STALL_AFTER_MS as i64 {
                return false;
            }
            match entry.last_attention_push {
                Some(pushed) => pushed,
                None => return false,
            }
        };
        let budget = self.call_budget(STALL_READ_BUDGET_MS);
        match self
            .store
            .mod_stall_oldest(seat, pushed, WATCH_BODY_LIMIT_BYTES, &budget)
        {
            Ok(Some(published)) => now.0.saturating_sub(published.0) > MOD_STALL_AFTER_MS as i64,
            Ok(None) | Err(_) => false,
        }
    }

    fn status(&self) -> Option<ModChannelStatus> {
        let state = self.lock();
        let mut channels: Vec<ModChannelEntry> = state
            .entries
            .iter()
            .map(|(seat, entry)| ModChannelEntry {
                seat: seat.clone(),
                harness: entry.harness.as_str().to_owned(),
                binding_generation: entry.generation,
                connected_since: entry.connected_since,
                state: entry.state,
            })
            .collect();
        channels.sort_by(|a, b| a.seat.cmp(&b.seat));
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
