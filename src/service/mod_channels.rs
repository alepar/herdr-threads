//! The daemon's registry of live push-channel mod watch channels (spec D2, D7).
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
    harness::adapter::IdleWake,
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
    presentation_lease: String,
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
    last_invitation_version: Option<u64>,
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
    presentation: super::delivery_presentation::PresentationState,
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
            presentation: super::delivery_presentation::PresentationState::default(),
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
            if let Some((channel, judged_ack)) = self.stalled_judgement(&seat, now) {
                self.close_inner(
                    &seat,
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

    /// One worker pass: re-reads every seat with a Live or ReconnectGrace
    /// entry, closes channels whose binding or seat state no longer holds, and
    /// pushes Attention where the seat's fingerprint changed. A store error is
    /// returned after every seat was tried.
    pub fn pass(&self, now: UtcMillis) -> Result<(), ApiError> {
        let seats: Vec<(SeatId, ModChannelId, u64, String)> = self
            .lock()
            .entries
            .iter()
            .filter(|(_, entry)| {
                matches!(
                    entry.state,
                    ModChannelState::Live | ModChannelState::ReconnectGrace
                )
            })
            .filter_map(|(seat, entry)| {
                Some((
                    seat.clone(),
                    entry.channel?,
                    entry.generation,
                    entry.native_session.clone(),
                ))
            })
            .collect();
        let budget = self.call_budget(2_000);
        let mut first_error = None;
        for (seat, channel, generation, native_session) in seats {
            let judged = Some(Judged { channel, ack: None });
            let view = match self.store.presentation_seat_view(&seat, &budget) {
                Ok(view) => view,
                Err(error) => {
                    first_error.get_or_insert(error);
                    continue;
                }
            };
            let Some(view) = view else {
                self.close_inner(&seat, judged, WatchCloseReason::Unresolved, now);
                continue;
            };
            if view.retired {
                self.close_inner(&seat, judged, WatchCloseReason::Retired, now);
                continue;
            }
            if !view.continuity_resolved {
                self.close_inner(&seat, judged, WatchCloseReason::Unresolved, now);
                continue;
            }
            let binding_holds = view.binding.as_ref().is_some_and(|binding| {
                binding.generation == generation && binding.native_session == native_session
            });
            if !binding_holds {
                self.close_inner(&seat, judged, WatchCloseReason::BindingChanged, now);
                continue;
            }
            let invitation_version =
                match self
                    .store
                    .presentation_refresh(&seat, generation, &native_session, &budget)
                {
                    Ok(Some(progress)) => {
                        // Continue finite maintenance on the next worker tick;
                        // it never wakes a seat or consumes presentation output.
                        if progress.more {
                            self.mark_dirty();
                        }
                        progress.version
                    }
                    Ok(None) => view.pending_invitation_version,
                    Err(error) => {
                        first_error.get_or_insert(error);
                        continue;
                    }
                };
            self.observe_fingerprint(
                &seat,
                generation,
                view.attention_version,
                view.fingerprint,
                invitation_version,
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
        seat: &SeatId,
        judged: Option<Judged>,
        reason: WatchCloseReason,
        now: UtcMillis,
    ) {
        let Ok(permit) = self.presentation.permit(seat) else {
            return;
        };
        let _owner = permit.lock().unwrap_or_else(|e| e.into_inner());
        let (kicked, sink) = {
            let mut state = self.lock();
            let Some(entry) = state.entries.get_mut(seat) else {
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
            let sink = entry.sink.take();
            let kicked = match reason {
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
            };
            (kicked, sink)
        };
        if let Some(sink) = sink {
            sink.push(WatchFrame::Close { reason });
        }
        if kicked {
            self.kick(1);
        }
    }

    /// The stall predicate (spec D7). When it holds, the channel id and the
    /// `last_ack_or_registration` it judged, so the close can verify that the
    /// entry is unchanged.
    fn stalled_judgement(
        &self,
        seat: &SeatId,
        now: UtcMillis,
    ) -> Option<(ModChannelId, UtcMillis)> {
        let (channel, judged_ack, pushed) = {
            let state = self.lock();
            let entry = state.entries.get(seat)?;
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
                entry.channel?,
                entry.last_ack_or_registration,
                entry.last_attention_push?,
            )
        };
        let budget = self.call_budget(STALL_READ_BUDGET_MS);
        match self
            .store
            .mod_stall_oldest(seat, pushed, WATCH_BODY_LIMIT_BYTES, &budget)
        {
            Ok(Some(published))
                if now.0.saturating_sub(published.0) > MOD_STALL_AFTER_MS as i64 =>
            {
                Some((channel, judged_ack))
            }
            _ => None,
        }
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
        invitation_version: u64,
        now: UtcMillis,
    ) {
        let (push, passive) = {
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
            let passive = entry
                .last_invitation_version
                .map_or(invitation_version != 0, |last| last != invitation_version);
            entry.last_invitation_version = Some(invitation_version);
            entry.last_fingerprint = Some(fingerprint);
            (
                changed && entry.state == ModChannelState::Live,
                passive && entry.state == ModChannelState::Live,
            )
        };
        if push {
            self.notify_at(seat, version, now);
        } else if passive {
            let state = self.lock();
            if let Some(entry) = state.entries.get(seat)
                && entry.generation == generation
                && entry.state == ModChannelState::Live
                && let Some(sink) = &entry.sink
            {
                // Existing wire signal requests a drain; it does not arm stall detection.
                sink.push(WatchFrame::Attention { version });
            }
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
    fn presentation_owner(&self, seat: &SeatId) -> Result<Option<Arc<Mutex<()>>>, ApiError> {
        self.presentation.permit(seat).map(Some)
    }
    fn presentation_lease(&self, channel: ModChannelId) -> Option<String> {
        self.lock()
            .entries
            .values()
            .find(|entry| entry.channel == Some(channel) && entry.state == ModChannelState::Live)
            .map(|entry| entry.presentation_lease.clone())
    }
    fn presentation_owner_live(
        &self,
        seat: &SeatId,
        generation: u64,
        native: &str,
        lease: &str,
    ) -> bool {
        self.lock().entries.get(seat).is_some_and(|entry| {
            entry.state == ModChannelState::Live
                && entry.generation == generation
                && entry.native_session == native
                && entry.presentation_lease == lease
        })
    }
    fn register(
        &self,
        registration: ModChannelRegistration,
        sink: Arc<dyn ModChannelSink>,
    ) -> Result<ModChannelId, WatchRefusalReason> {
        let permit = self
            .presentation
            .permit(&registration.seat)
            .map_err(|_| WatchRefusalReason::Busy)?;
        let _owner = permit.lock().unwrap_or_else(|e| e.into_inner());
        let now = registration.registered_at;
        let (id, old_sink) = {
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
            let old_sink = state
                .entries
                .get(&registration.seat)
                .filter(|old| old.state == ModChannelState::Live)
                .and_then(|old| old.sink.clone());
            let id = ModChannelId(state.next_id);
            state.next_id += 1;
            state.entries.insert(
                registration.seat.clone(),
                Entry {
                    channel: Some(id),
                    presentation_lease: uuid::Uuid::new_v4().simple().to_string(),
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
                    last_invitation_version: None,
                },
            );
            (id, old_sink)
        };
        if let Some(old_sink) = old_sink {
            old_sink.push(WatchFrame::Close {
                reason: WatchCloseReason::Replaced,
            });
        }
        (self.log)(&format!(
            "mod channel registered seat={} generation={} provenance={}",
            registration.seat.as_str(),
            registration.binding_generation,
            crate::protocol::authority::COOPERATIVE_MOD_CHANNEL_PROVENANCE
        ));
        self.dirty.store(true, Ordering::SeqCst);
        self.wake_worker();
        Ok(id)
    }

    fn unregister(&self, channel: ModChannelId, now: UtcMillis) {
        let seat = self
            .lock()
            .entries
            .iter()
            .find(|(_, entry)| entry.channel == Some(channel))
            .map(|(seat, _)| seat.clone());
        let Some(seat) = seat else {
            return;
        };
        let Ok(permit) = self.presentation.permit(&seat) else {
            return;
        };
        let _owner = permit.lock().unwrap_or_else(|e| e.into_inner());
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
        self.close_inner(seat, None, reason, now);
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

    fn seat_live(&self, seat: &SeatId) -> bool {
        let now = self.clock.utc_now();
        self.lock()
            .entries
            .get(seat)
            .is_some_and(|entry| entry.grace_until.is_none_or(|until| until > now))
    }

    fn wake_suppressed(&self, seat: &SeatId) -> bool {
        let now = self.clock.utc_now();
        self.lock().entries.get(seat).is_some_and(|entry| {
            entry.idle_wake == IdleWake::Channel
                && entry.grace_until.is_none_or(|until| until > now)
        })
    }

    fn record_attention_push(&self, seat: &SeatId, binding_generation: u64, now: UtcMillis) {
        if let Some(entry) = self.lock().entries.get_mut(seat)
            && entry.generation == binding_generation
            && matches!(
                entry.state,
                ModChannelState::Live | ModChannelState::ReconnectGrace
            )
        {
            entry.last_attention_push = Some(now);
        }
    }

    fn record_presentation_attention_push(
        &self,
        seat: &SeatId,
        binding_generation: u64,
        native_session: &str,
        lease: &str,
        now: UtcMillis,
    ) {
        // The comparison and write share the registry entry lock; disconnect
        // or replacement cannot turn an old confirmation into new evidence.
        if let Some(entry) = self.lock().entries.get_mut(seat)
            && entry.state == ModChannelState::Live
            && entry.generation == binding_generation
            && entry.native_session == native_session
            && entry.presentation_lease == lease
        {
            entry.last_attention_push = Some(now);
        }
    }

    fn record_ack(&self, seat: &SeatId, binding_generation: u64, now: UtcMillis) {
        if let Some(entry) = self.lock().entries.get_mut(seat)
            && entry.generation == binding_generation
            && matches!(
                entry.state,
                ModChannelState::Live | ModChannelState::ReconnectGrace
            )
        {
            entry.last_ack_or_registration = now;
        }
    }

    fn stalled(&self, seat: &SeatId, now: UtcMillis) -> bool {
        self.stalled_judgement(seat, now).is_some()
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

#[cfg(test)]
#[path = "../../tests/delivery/push_channel.rs"]
mod push_channel_tests;
