//! One-at-a-time writer admission shared by foreground calls and background quanta.

use super::workers::{budget_error, error};
use crate::protocol::{
    results::{ApiError, ErrorCode},
    time::{CallBudget, Clock},
};
use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex},
    time::Duration,
};

#[derive(Debug, Default)]
struct WriterState {
    active: bool,
    next_ticket: u64,
    foreground: VecDeque<u64>,
    background: VecDeque<u64>,
    foreground_since_background: u8,
}

/// One writer admission at a time. Waiting foreground calls retain FIFO order.
/// A ready background quantum follows at most eight foreground decisions;
/// foreground resumes after that single quantum.
#[derive(Debug)]
pub struct FairWriter {
    queued_limit: usize,
    state: Mutex<WriterState>,
    changed: Condvar,
}

#[derive(Debug, Clone, Copy)]
enum WriterClass {
    Foreground,
    Background,
}

#[derive(Debug)]
pub struct WriterGuard<'a> {
    lane: &'a FairWriter,
    class: WriterClass,
}

impl FairWriter {
    pub fn new(queued_limit: usize) -> Self {
        Self {
            queued_limit,
            state: Mutex::new(WriterState::default()),
            changed: Condvar::new(),
        }
    }

    pub fn waiting(&self) -> (usize, usize) {
        let state = self.state.lock().expect("writer lane lock poisoned");
        (state.foreground.len(), state.background.len())
    }

    pub fn enter_foreground(
        &self,
        budget: &CallBudget,
        clock: &dyn Clock,
    ) -> Result<WriterGuard<'_>, ApiError> {
        self.enter(WriterClass::Foreground, budget, clock)
    }

    pub fn enter_background(
        &self,
        budget: &CallBudget,
        clock: &dyn Clock,
    ) -> Result<WriterGuard<'_>, ApiError> {
        self.enter(WriterClass::Background, budget, clock)
    }

    fn enter(
        &self,
        class: WriterClass,
        budget: &CallBudget,
        clock: &dyn Clock,
    ) -> Result<WriterGuard<'_>, ApiError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| error(ErrorCode::StoreCorrupt, "writer lane lock poisoned"))?;
        if budget.is_exhausted(clock) {
            return Err(budget_error(budget));
        }
        if state.foreground.len() + state.background.len() >= self.queued_limit {
            return Err(error(ErrorCode::StoreBusy, "writer admission full"));
        }
        let ticket = state.next_ticket;
        state.next_ticket = state
            .next_ticket
            .checked_add(1)
            .ok_or_else(|| error(ErrorCode::StoreBusy, "writer admission counter exhausted"))?;
        match class {
            WriterClass::Foreground => state.foreground.push_back(ticket),
            WriterClass::Background => state.background.push_back(ticket),
        }
        loop {
            if budget.is_exhausted(clock) {
                let queue = match class {
                    WriterClass::Foreground => &mut state.foreground,
                    WriterClass::Background => &mut state.background,
                };
                if let Some(position) = queue.iter().position(|queued| *queued == ticket) {
                    queue.remove(position);
                }
                self.changed.notify_all();
                return Err(budget_error(budget));
            }
            let ready = match class {
                WriterClass::Foreground => {
                    state.foreground.front() == Some(&ticket)
                        && (state.background.is_empty() || state.foreground_since_background < 8)
                }
                WriterClass::Background => {
                    state.background.front() == Some(&ticket)
                        && (state.foreground.is_empty() || state.foreground_since_background >= 8)
                }
            };
            if !state.active && ready {
                match class {
                    WriterClass::Foreground => {
                        state.foreground.pop_front();
                    }
                    WriterClass::Background => {
                        state.background.pop_front();
                    }
                }
                state.active = true;
                return Ok(WriterGuard { lane: self, class });
            }
            state = self
                .changed
                .wait_timeout(state, Duration::from_millis(10))
                .map_err(|_| error(ErrorCode::StoreCorrupt, "writer lane lock poisoned"))?
                .0;
        }
    }
}

impl Drop for WriterGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.lane.state.lock() {
            state.active = false;
            match self.class {
                WriterClass::Foreground => {
                    state.foreground_since_background =
                        state.foreground_since_background.saturating_add(1).min(8);
                }
                WriterClass::Background => state.foreground_since_background = 0,
            }
            self.lane.changed.notify_all();
        }
    }
}
