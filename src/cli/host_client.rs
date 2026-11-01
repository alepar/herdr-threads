//! The CLI-side launch handle: the one place `launch` and `handoff` obtain their host from.
//!
//! Responsibility (LaunchHandle)
//! - Owns: building the selected host's adapter for an endpoint and handing out its
//!   [`CreateTargetPort`] (target creation) and its Herdr pane operations (labels, guarded
//!   start, topology reads) as separate views of the one connection.
//! - Hides: which adapter type answers; creation code holds a `&dyn CreateTargetPort`.
//! - Must not: create anything itself, write the journal or the store, or branch on the host
//!   type in launch and handoff code.
//! - Role: handle over the host seam. A new host's launch arrives as another constructor here
//!   plus its [`CreateTargetPort`] impl; callers stay as they are.

use std::{path::PathBuf, sync::Arc};

use crate::{host::native::NativeCli, ports::CreateTargetPort, protocol::time::Clock};

pub struct LaunchHandle {
    herdr: NativeCli,
}

impl LaunchHandle {
    /// The Herdr server on `endpoint`.
    pub fn herdr(endpoint: PathBuf, clock: Arc<dyn Clock>) -> Self {
        Self {
            herdr: crate::host::target_creator::herdr_host(endpoint, clock),
        }
    }

    /// Creation of launch and handoff targets.
    pub fn create_port(&self) -> &dyn CreateTargetPort {
        &self.herdr
    }

    /// The Herdr pane operations (`launch --pane`, labels, topology) on the same endpoint.
    pub fn host(&self) -> &NativeCli {
        &self.herdr
    }
}
