//! Queue control is separate from hook delivery; the core selects the wake transport.
pub(super) fn replay_scope_for_session(
    session: &crate::protocol::ids::NativeSessionId,
) -> Option<crate::harness::wake::WakeReplayScope> {
    let id = uuid::Uuid::parse_str(session.as_str()).ok()?;
    if id.to_string() != session.as_str() {
        return None;
    }
    crate::harness::wake::WakeReplayScope::parse(format!("session:{id}")).ok()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod capture;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod control;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod helper;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod reap;
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod rules;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod implementation {
    use super::{capture, control};
    use crate::{
        harness::wake::{
            ControlFailure, OwnedWakeEntries, WakeAddOutcome, WakeControl, WakeControlContext,
            WakeControlTarget, WakeScope,
        },
        protocol::results::ApiError,
    };
    use std::{
        collections::BTreeSet,
        sync::Mutex,
        time::{Duration, Instant},
    };
    pub struct Control;
    static ACTIVE: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
    struct Active(String);
    impl Drop for Active {
        fn drop(&mut self) {
            if let Ok(mut active) = ACTIVE.lock() {
                active.remove(&self.0);
            }
        }
    }
    fn enter(t: &WakeControlTarget) -> Result<Active, ApiError> {
        let key = format!(
            "{}:{}:{}",
            t.scope.process.pid,
            t.scope.process.start,
            t.scope.native_session.as_str()
        );
        if !ACTIVE
            .lock()
            .map_err(|_| control::error("queue control unavailable"))?
            .insert(key.clone())
        {
            return Err(control::error("queue control already active"));
        }
        Ok(Active(key))
    }
    fn observe(scope: &WakeScope, c: &WakeControlContext<'_>) -> Option<capture::Descriptor> {
        if c.budget.is_exhausted(c.clock)
            || c.processes.kernel_boot().as_ref() != Some(&scope.kernel_boot)
            || scope.kernel_boot.is_empty()
            || !scope.process.is_alive(c.processes)
        {
            return None;
        }
        let left = c
            .budget
            .deadline
            .0
            .saturating_sub(c.clock.monotonic_now().0)
            .min(100);
        let d = capture::process_files(
            scope.process.pid,
            scope.native_session.as_str(),
            Instant::now() + Duration::from_millis(left),
        )?;
        if c.budget.is_exhausted(c.clock)
            || !scope.process.is_alive(c.processes)
            || c.processes.kernel_boot().as_ref() != Some(&scope.kernel_boot)
        {
            return None;
        }
        Some(d)
    }
    fn saved_descriptor(
        t: &WakeControlTarget,
        c: &WakeControlContext<'_>,
    ) -> Result<capture::Descriptor, ApiError> {
        if c.budget.is_exhausted(c.clock) || t.descriptor.len() > 16384 {
            return Err(control::error("queue descriptor unavailable"));
        }
        let d: capture::Descriptor = serde_json::from_slice(&t.descriptor)
            .map_err(|_| control::error("invalid queue descriptor"))?;
        if d.database
            .path
            .file_name()
            .is_none_or(|v| v != "queue_1.sqlite")
            || capture::identity(&d.database.path).as_ref() != Some(&d.database)
            || capture::identity(&d.executable.path).as_ref() != Some(&d.executable)
        {
            return Err(control::error("saved queue storage or executable changed"));
        }
        Ok(d)
    }
    fn validate(
        t: &WakeControlTarget,
        c: &WakeControlContext<'_>,
    ) -> Result<capture::Descriptor, ApiError> {
        let d = saved_descriptor(t, c)?;
        if observe(&t.scope, c).as_ref() != Some(&d) {
            return Err(control::error("queue receiver changed"));
        }
        Ok(d)
    }
    impl WakeControl for Control {
        fn reap_owned_helpers(&self) {
            super::reap::OWNED.reap();
        }
        fn has_owned_helpers(&self) -> bool {
            super::reap::OWNED.pending()
        }
        fn probe_helper(
            &self,
            witness: &crate::harness::wake::WakeHelperWitness,
            context: &WakeControlContext<'_>,
        ) -> Result<crate::harness::wake::WakeHelperPresence, ApiError> {
            super::helper::probe(witness, context)
        }

        fn replay_scope(
            &self,
            target: &WakeControlTarget,
        ) -> Result<crate::harness::wake::WakeReplayScope, ApiError> {
            self.replay_scope_for_session(&target.scope.native_session)
        }
        fn replay_scope_for_session(
            &self,
            native_session: &crate::protocol::ids::NativeSessionId,
        ) -> Result<crate::harness::wake::WakeReplayScope, ApiError> {
            super::replay_scope_for_session(native_session)
                .ok_or_else(|| control::error("invalid native wake session"))
        }

        fn observe(
            &self,
            scope: &WakeScope,
            c: &WakeControlContext<'_>,
        ) -> Result<Option<WakeControlTarget>, ApiError> {
            let Some(d) = observe(scope, c) else {
                return Ok(None);
            };
            let descriptor =
                serde_json::to_vec(&d).map_err(|_| control::error("invalid queue descriptor"))?;
            if descriptor.len() > 16384 {
                return Ok(None);
            }
            Ok(Some(WakeControlTarget {
                scope: scope.clone(),
                descriptor,
            }))
        }
        fn add(
            &self,
            t: &WakeControlTarget,
            correlation: &str,
            authorize: &mut dyn FnMut() -> Result<bool, ApiError>,
            c: &WakeControlContext<'_>,
        ) -> WakeAddOutcome {
            let _active = match enter(t) {
                Ok(v) => v,
                Err(e) => return WakeAddOutcome::RefusedBeforeEffect(e.into()),
            };
            let d = match validate(t, c) {
                Ok(v) => v,
                Err(e) => return WakeAddOutcome::RefusedBeforeEffect(e.into()),
            };
            control::enqueue(
                &d,
                t.scope.native_session.as_str(),
                correlation,
                &mut || validate(t, c).map(|_| ()),
                authorize,
                c,
            )
        }
        fn list(
            &self,
            t: &WakeControlTarget,
            correlation: &str,
            c: &WakeControlContext<'_>,
        ) -> Result<OwnedWakeEntries, Box<ControlFailure>> {
            let _active = enter(t)?;
            let d = saved_descriptor(t, c)?;
            control::list_entries(&d, t.scope.native_session.as_str(), correlation, c)
        }
        fn delete(
            &self,
            t: &WakeControlTarget,
            id: &str,
            c: &WakeControlContext<'_>,
        ) -> Result<bool, Box<ControlFailure>> {
            let _active = enter(t)?;
            let d = saved_descriptor(t, c)?;
            control::delete_entry(&d, t.scope.native_session.as_str(), id, c)
        }
    }
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub static CONTROL: implementation::Control = implementation::Control;
