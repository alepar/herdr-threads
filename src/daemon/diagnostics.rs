//! Bounded diagnostic delivery owned by the elected daemon child.

use std::collections::VecDeque;
use std::io::{self, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSource {
    Stdout,
    Stderr,
    Daemon,
}

/// Limits apply before a fragment can enter the child-owned queue.
#[derive(Debug, Clone, Copy)]
pub struct BufferLimits {
    max_buffer_bytes: usize,
    max_fragment_bytes: usize,
}

impl BufferLimits {
    pub fn new(max_buffer_bytes: usize, max_fragment_bytes: usize) -> io::Result<Self> {
        if max_buffer_bytes == 0 || max_fragment_bytes == 0 || max_fragment_bytes > max_buffer_bytes
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid diagnostic limits",
            ));
        }
        Ok(Self {
            max_buffer_bytes,
            max_fragment_bytes,
        })
    }
    pub fn max_fragment_bytes(&self) -> usize {
        self.max_fragment_bytes
    }
}

/// `install` runs only after election. `drain` and `close` run before the owner lease drops.
pub trait DiagnosticSink: Send {
    fn install(&mut self) -> io::Result<()>;
    fn write(&mut self, source: DiagnosticSource, fragment: &[u8]) -> io::Result<()>;
    fn drain(&mut self) -> io::Result<()>;
    fn close(&mut self) -> io::Result<()>;
}

/// A simple writer sink for the later rotating-log integration.
pub struct WriterSink<W: Write + Send>(pub W);

impl<W: Write + Send> DiagnosticSink for WriterSink<W> {
    fn install(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn write(&mut self, _source: DiagnosticSource, fragment: &[u8]) -> io::Result<()> {
        self.0.write_all(fragment)
    }
    fn drain(&mut self) -> io::Result<()> {
        self.0.flush()
    }
    fn close(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) struct DiagnosticBuffer<S: DiagnosticSink> {
    sink: S,
    limits: BufferLimits,
    pending: VecDeque<(DiagnosticSource, Vec<u8>)>,
    pending_bytes: usize,
    dropped_bytes: usize,
    failed: bool,
    closed: bool,
}

impl<S: DiagnosticSink> DiagnosticBuffer<S> {
    pub(crate) fn install(mut sink: S, limits: BufferLimits) -> io::Result<Self> {
        sink.install()?;
        Ok(Self {
            sink,
            limits,
            pending: VecDeque::new(),
            pending_bytes: 0,
            dropped_bytes: 0,
            failed: false,
            closed: false,
        })
    }

    pub(crate) fn emit(&mut self, source: DiagnosticSource, fragment: &[u8]) -> io::Result<()> {
        if self.closed {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "diagnostic sink closed",
            ));
        }
        if self.failed {
            self.discard(fragment.len());
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "diagnostic sink failed",
            ));
        }
        let retained = fragment.len().min(self.limits.max_fragment_bytes);
        self.dropped_bytes = self.dropped_bytes.saturating_add(fragment.len() - retained);
        if retained == 0 {
            return Ok(());
        }
        while retained > self.limits.max_buffer_bytes - self.pending_bytes {
            if let Some((_, removed)) = self.pending.pop_front() {
                self.pending_bytes -= removed.len();
                self.dropped_bytes = self.dropped_bytes.saturating_add(removed.len());
            }
        }
        self.pending
            .push_back((source, fragment[..retained].to_vec()));
        self.pending_bytes += retained;
        Ok(())
    }

    pub(crate) fn buffered_bytes(&self) -> usize {
        self.pending_bytes
    }
    pub(crate) fn dropped_bytes(&self) -> usize {
        self.dropped_bytes
    }

    pub(crate) fn max_fragment_bytes(&self) -> usize {
        self.limits.max_fragment_bytes()
    }

    pub(crate) fn discard(&mut self, bytes: usize) {
        self.dropped_bytes = self.dropped_bytes.saturating_add(bytes);
    }

    /// Deliver pending fragments during the owner loop, with no unbounded staging.
    pub(crate) fn flush(&mut self) -> io::Result<()> {
        if self.closed || self.failed {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "diagnostic sink unavailable",
            ));
        }
        while let Some((source, fragment)) = self.pending.pop_front() {
            self.pending_bytes -= fragment.len();
            if let Err(error) = self.sink.write(source, &fragment) {
                self.discard(fragment.len() + self.pending_bytes);
                self.pending.clear();
                self.pending_bytes = 0;
                self.failed = true;
                return Err(error);
            }
        }
        if let Err(error) = self.sink.drain() {
            self.failed = true;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn drain_and_close(&mut self) -> io::Result<()> {
        if self.closed {
            return Ok(());
        }
        let was_failed = self.failed;
        let mut result = if was_failed {
            self.sink.drain()
        } else {
            self.flush()
        };
        if !was_failed && result.is_err() {
            // A later write can fail after earlier fragments entered the sink.
            // Preserve the first error, but flush those earlier writes before closing.
            let _ = self.sink.drain();
        }
        self.closed = true;
        if let Err(error) = self.sink.close()
            && result.is_ok()
        {
            result = Err(error);
        }
        result
    }
}

#[cfg(test)]
#[path = "../../tests/daemon/diagnostics_contract.rs"]
mod tests;
