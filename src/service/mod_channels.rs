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
//! [`MOD_STALL_AFTER_MS`], an Attention push on record, and a non-truncated
//! wake-class pending item (an ordinary receipt or a delivery row the seat's
//! nudge threshold admits) published before that push and itself older than
//! [`MOD_STALL_AFTER_MS`].
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
        ModBindingRef, ModChannelId, ModChannelMode, ModChannelRegistration, ModChannelSink,
        ModChannels, ModFingerprint, ModStoreReads, WatchReplayScope, WatchReplaySnapshot,
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
    mode: ModChannelMode,
    pending_delivery_recheck: bool,
    presentation_lease: Option<String>,
    owner_replay: Option<WatchReplaySnapshot>,
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

struct ChannelWork {
    binding: BindingId,
    seat: SeatId,
    channel: ModChannelId,
    mode: ModChannelMode,
    native_session: String,
    delivery_recheck: bool,
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

#[cfg(test)]
type BeforeFinalPush = Box<dyn FnOnce(&BindingId, ModChannelId, bool) + Send>;

pub struct ModChannelRegistry {
    clock: Arc<dyn Clock>,
    store: Arc<dyn ModStoreReads>,
    instance: String,
    kick_wakes: Arc<dyn Fn() + Send + Sync>,
    log: Arc<dyn Fn(&str) + Send + Sync>,
    wake_worker: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    dirty: AtomicBool,
    #[cfg(test)]
    before_final_push: Mutex<Option<BeforeFinalPush>>,
    /// Binding budget of `register`; production keeps `MAX_WATCH_CONNECTIONS`.
    cap: usize,
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
            #[cfg(test)]
            before_final_push: Mutex::new(None),
            cap: MAX_WATCH_CONNECTIONS,
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

    /// Lower the binding budget (never above `MAX_WATCH_CONNECTIONS`).
    #[must_use]
    pub fn with_connection_cap(mut self, cap: usize) -> Self {
        debug_assert!((1..=MAX_WATCH_CONNECTIONS).contains(&cap));
        self.cap = cap.min(MAX_WATCH_CONNECTIONS);
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
        let entries: Vec<ChannelWork> = self
            .lock()
            .entries
            .iter_mut()
            .filter(|(_, entry)| {
                matches!(
                    entry.state,
                    ModChannelState::Live | ModChannelState::ReconnectGrace
                )
            })
            .filter_map(|(binding, entry)| {
                let channel = entry.channel?;
                let delivery_recheck = entry.mode == ModChannelMode::PresentationV2
                    && entry.state == ModChannelState::Live
                    && std::mem::take(&mut entry.pending_delivery_recheck);
                Some(ChannelWork {
                    binding: binding.clone(),
                    seat: entry.seat.clone(),
                    channel,
                    mode: entry.mode,
                    native_session: entry.native_session.clone(),
                    delivery_recheck,
                })
            })
            .collect();
        let budget = self.call_budget(2_000);
        let mut first_error = None;
        for work in entries {
            let ChannelWork {
                binding,
                seat,
                channel,
                native_session,
                ..
            } = &work;
            let channel = *channel;
            let judged = Some(Judged { channel, ack: None });
            let view = match match work.mode {
                ModChannelMode::Legacy => {
                    self.store
                        .mod_binding_view(seat, ModBindingRef::Id(binding), &budget)
                }
                ModChannelMode::PresentationV2 => {
                    self.store
                        .presentation_seat_view(seat, ModBindingRef::Id(binding), &budget)
                }
            } {
                Ok(view) => view,
                Err(error) => {
                    self.restore_delivery_recheck(&work, work.delivery_recheck);
                    first_error.get_or_insert(error);
                    continue;
                }
            };
            let Some(view) = view else {
                self.close_inner(binding, judged, WatchCloseReason::Unresolved, now);
                continue;
            };
            if view.retired {
                self.close_inner(binding, judged, WatchCloseReason::Retired, now);
                continue;
            }
            if !view.continuity_resolved {
                self.close_inner(binding, judged, WatchCloseReason::Unresolved, now);
                continue;
            }
            let binding_holds = view.binding.as_ref().is_some_and(|held| {
                &held.binding_id == binding
                    && held.holds_channel()
                    && &held.native_session == native_session
            });
            if !binding_holds {
                self.close_inner(binding, judged, WatchCloseReason::BindingChanged, now);
                continue;
            }
            let invitation_version = if work.mode == ModChannelMode::Legacy {
                view.pending_invitation_version
            } else {
                match self.store.presentation_refresh(
                    seat,
                    binding,
                    view.binding
                        .as_ref()
                        .expect("binding checked above")
                        .generation,
                    native_session,
                    &budget,
                ) {
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
                        self.restore_delivery_recheck(&work, work.delivery_recheck);
                        first_error.get_or_insert(error);
                        continue;
                    }
                }
            };
            self.observe_fingerprint(
                &work,
                view.attention_version,
                view.fingerprint,
                invitation_version,
                now,
            );
        }
        first_error.map_or(Ok(()), Err)
    }

    fn restore_delivery_recheck(&self, work: &ChannelWork, pending: bool) {
        if pending
            && work.mode == ModChannelMode::PresentationV2
            && let Some(entry) = self.lock().entries.get_mut(&work.binding)
            && entry.channel == Some(work.channel)
            && entry.mode == work.mode
            && entry.state == ModChannelState::Live
        {
            entry.pending_delivery_recheck = true;
        }
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
        let Some(seat) = self
            .lock()
            .entries
            .get(binding)
            .map(|entry| entry.seat.clone())
        else {
            return;
        };
        let Ok(permit) = self.presentation.permit(&seat) else {
            return;
        };
        let _owner = permit.lock().unwrap_or_else(|e| e.into_inner());
        let (kicked, sink) = {
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
                    state.remove(binding);
                    if reason == WatchCloseReason::Stalled {
                        state.cooldowns.insert(
                            binding.clone(),
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
        work: &ChannelWork,
        version: u64,
        fingerprint: ModFingerprint,
        invitation_version: u64,
        now: UtcMillis,
    ) {
        let binding = &work.binding;
        let channel = work.channel;
        let (push, passive) = {
            let mut state = self.lock();
            let Some(entry) = state.entries.get_mut(binding) else {
                return;
            };
            if entry.channel != Some(channel) || entry.mode != work.mode {
                return;
            }
            let changed = match &entry.last_fingerprint {
                None => fingerprint.has_pending(),
                Some(last) => *last != fingerprint,
            };
            let passive = work.delivery_recheck
                || entry
                    .last_invitation_version
                    .map_or(invitation_version != 0, |last| last != invitation_version);
            entry.last_invitation_version = Some(invitation_version);
            entry.last_fingerprint = Some(fingerprint);
            (
                changed && entry.state == ModChannelState::Live,
                passive && entry.state == ModChannelState::Live,
            )
        };
        #[cfg(test)]
        if push || passive {
            // The first state guard is gone. Take the per-registry hook out of
            // its mutex before the test performs a real channel replacement.
            let callback = self.before_final_push.lock().unwrap().take();
            if let Some(callback) = callback {
                callback(binding, channel, push);
            }
        }
        if push {
            if !self.notify_binding(binding, channel, work.mode, version, now) {
                self.restore_delivery_recheck(work, true);
            }
        } else if passive {
            let mut state = self.lock();
            if let Some(entry) = state.entries.get_mut(binding)
                && entry.channel == Some(channel)
                && entry.mode == work.mode
                && entry.state == ModChannelState::Live
                && let Some(sink) = entry.sink.clone()
            {
                // Existing wire signal requests a drain; it does not arm stall detection.
                if !sink.push(WatchFrame::Attention { version })
                    && entry.mode == ModChannelMode::PresentationV2
                {
                    // False means a closed receiver. The safety pass or reconnect
                    // retries; immediately dirtying this entry would spin.
                    entry.pending_delivery_recheck = true;
                }
            }
        }
    }

    fn notify_binding(
        &self,
        binding: &BindingId,
        channel: ModChannelId,
        mode: ModChannelMode,
        version: u64,
        now: UtcMillis,
    ) -> bool {
        // The caller's `now` can predate the store read whose fingerprint this push announces (the worker reads
        // its clock before `pass`). The stall rule counts only items published at or before the recorded push,
        // so the record is taken no earlier than the push itself (ht-yha.58).
        let pushed_at = UtcMillis(now.0.max(self.clock.utc_now().0));
        let mut state = self.lock();
        let Some(entry) = state.entries.get_mut(binding) else {
            return false;
        };
        if entry.channel != Some(channel)
            || entry.mode != mode
            || entry.state != ModChannelState::Live
        {
            return false;
        }
        let Some(sink) = entry.sink.clone() else {
            return false;
        };
        if sink.push(WatchFrame::Attention { version }) {
            entry.last_attention_push = Some(pushed_at);
            true
        } else {
            if mode == ModChannelMode::PresentationV2 {
                entry.pending_delivery_recheck = true;
            }
            false
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
            .find(|entry| {
                entry.channel == Some(channel)
                    && entry.mode == ModChannelMode::PresentationV2
                    && entry.state == ModChannelState::Live
            })
            .and_then(|entry| entry.presentation_lease.clone())
    }
    fn presentation_owner_live(
        &self,
        seat: &SeatId,
        binding: &BindingId,
        generation: u64,
        native: &str,
        lease: &str,
    ) -> bool {
        self.lock().entries.get(binding).is_some_and(|entry| {
            &entry.seat == seat
                && entry.mode == ModChannelMode::PresentationV2
                && entry.state == ModChannelState::Live
                && entry.generation == generation
                && entry.native_session == native
                && entry.presentation_lease.as_deref() == Some(lease)
        })
    }
    fn presentation_replay_snapshot(
        &self,
        claim: &crate::protocol::authority::CallerClaim,
        lease: &str,
        known: &crate::delivery::ledger::OrdinaryDueSnapshot,
    ) -> Option<WatchReplaySnapshot> {
        let state = self.lock();
        let entry = state.entries.get(&known.binding_id)?;
        if entry.seat != claim.seat
            || entry.mode != ModChannelMode::PresentationV2
            || entry.harness != claim.harness
            || entry.state != ModChannelState::Live
            || entry.generation != claim.binding_generation
            || entry.native_session != claim.native_session.as_str()
            || entry.presentation_lease.as_deref() != Some(lease)
            || known.generation != claim.binding_generation
        {
            return None;
        }
        let scope = WatchReplayScope {
            seat: claim.seat.clone(),
            binding_id: known.binding_id.clone(),
            generation: claim.binding_generation,
            native_session: claim.native_session.as_str().to_owned(),
            execution: claim.execution.clone(),
            lease: lease.to_owned(),
            incarnation: known.incarnation,
        };
        // Capture transiently. Even an empty successful capture is terminal for
        // this lease; an error must leave the Entry uninitialized.
        let mut snapshot = match &entry.owner_replay {
            None => WatchReplaySnapshot {
                scope,
                remaining: known.stamps.clone(),
                active: !known.stamps.is_empty(),
            },
            Some(saved) => WatchReplaySnapshot {
                remaining: if saved.scope == scope {
                    saved.remaining.clone()
                } else {
                    Default::default()
                },
                active: !saved.remaining.is_empty(),
                scope,
            },
        };
        snapshot
            .remaining
            .retain(|id, stamp| known.stamps.get(id) == Some(stamp));
        Some(snapshot)
    }
    fn apply_presentation_replay(
        &self,
        mut snapshot: WatchReplaySnapshot,
        completed: &std::collections::BTreeMap<
            crate::protocol::ids::MessageId,
            crate::delivery::ledger::Offer,
        >,
        current: &crate::delivery::ledger::OrdinaryDueSnapshot,
    ) {
        // A current snapshot from a sibling is never progress for this owner.
        // Incarnation loss within this same binding still abandons its pass below.
        if current.binding_id != snapshot.scope.binding_id {
            return;
        }
        let mut state = self.lock();
        let Some(entry) = state.entries.get_mut(&snapshot.scope.binding_id) else {
            return;
        };
        if entry.seat != snapshot.scope.seat
            || entry.mode != ModChannelMode::PresentationV2
            || entry.state != ModChannelState::Live
            || entry.generation != snapshot.scope.generation
            || entry.native_session != snapshot.scope.native_session
            || entry.presentation_lease.as_deref() != Some(snapshot.scope.lease.as_str())
        {
            return;
        }
        let saved = entry.owner_replay.get_or_insert_with(|| {
            snapshot.active = false;
            snapshot.clone()
        });
        if saved.scope != snapshot.scope
            || saved.scope.incarnation != current.incarnation
            || saved.scope.generation != current.generation
            || saved.scope.binding_id != current.binding_id
        {
            // Incarnation loss abandons the pass, never recaptures in this lease.
            saved.remaining.clear();
            return;
        }
        saved.remaining.retain(|id, stamp| {
            current.stamps.get(id) == Some(stamp) && completed.get(id) != Some(stamp)
        });
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
                .get(&registration.binding)
                .is_some_and(|until| *until > now)
            {
                return Err(WatchRefusalReason::Cooldown);
            }
            if !state.entries.contains_key(&registration.binding) && state.entries.len() >= self.cap
            {
                return Err(WatchRefusalReason::Busy);
            }
            let old_sink = state
                .entries
                .get(&registration.binding)
                .filter(|old| old.state == ModChannelState::Live)
                .and_then(|old| old.sink.clone());
            let id = ModChannelId(state.next_id);
            state.next_id += 1;
            state.insert(
                registration.binding.clone(),
                Entry {
                    seat: registration.seat.clone(),
                    channel: Some(id),
                    mode: registration.mode,
                    pending_delivery_recheck: false,
                    owner_replay: None,
                    presentation_lease: (registration.mode == ModChannelMode::PresentationV2)
                        .then(|| uuid::Uuid::new_v4().simple().to_string()),
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
        let seat = self
            .lock()
            .entries
            .iter()
            .find(|(_, entry)| entry.channel == Some(channel))
            .map(|(_, entry)| entry.seat.clone());
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
        let channels: Vec<_> = {
            let state = self.lock();
            state
                .seat_bindings(seat)
                .into_iter()
                .filter_map(|binding| {
                    let entry = state.entries.get(&binding)?;
                    Some((binding, entry.channel?, entry.mode))
                })
                .collect()
        };
        for (binding, channel, mode) in channels {
            self.notify_binding(&binding, channel, mode, attention_version, now);
        }
    }

    fn presentation_delivery_may_have_changed(&self, seat: &SeatId) {
        let marked = {
            let mut state = self.lock();
            let bindings = state.seat_bindings(seat);
            let mut marked = false;
            for binding in bindings {
                if let Some(entry) = state.entries.get_mut(&binding)
                    && entry.mode == ModChannelMode::PresentationV2
                    && entry.state == ModChannelState::Live
                {
                    entry.pending_delivery_recheck = true;
                    marked = true;
                }
            }
            marked
        };
        if marked {
            self.mark_dirty();
            self.wake_worker();
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

    fn record_presentation_attention_push(
        &self,
        seat: &SeatId,
        binding: &BindingId,
        binding_generation: u64,
        native_session: &str,
        lease: &str,
        now: UtcMillis,
    ) {
        // The comparison and write share the registry entry lock; disconnect
        // or replacement cannot turn an old confirmation into new evidence.
        if let Some(entry) = self.lock().entries.get_mut(binding)
            && &entry.seat == seat
            && entry.mode == ModChannelMode::PresentationV2
            && entry.state == ModChannelState::Live
            && entry.generation == binding_generation
            && entry.native_session == native_session
            && entry.presentation_lease.as_deref() == Some(lease)
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
